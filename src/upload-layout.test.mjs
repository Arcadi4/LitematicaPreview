import assert from "node:assert/strict"
import { test } from "vite-plus/test"
import { ARENA_GROUP_BYTES, buildUploadLayout, UPLOAD_CHUNK } from "./upload-layout.ts"

function fixture(count = 16) {
  const bytes = new Map()
  const parts = Array.from({ length: count }, (_, index) => {
    const arrays = [
      new Float32Array([index, 1, 2, index, 3, 4, index, 5, 6]),
      new Int8Array([1, -1, 0, 0, 1, -1, -1, 0, 1]),
      new Float32Array([0, 0, 1, 0, 0, 1]),
      new Uint8Array([255, index, 0, 255, 255, index, 0, 255, 255, index, 0, 255]),
      new Uint32Array([0, 1, 2]),
    ]
    const buffers = arrays.map((array, attribute) => {
      const id = index * 5 + attribute
      bytes.set(id, new Uint8Array(array.buffer))
      return id
    })
    return {
      vertexCount: 3,
      indexCount: 3,
      buffers,
      textureIndex: index % 3 === 1 ? 1 : 0,
      alphaMode: 2,
    }
  })
  return {
    metadata: {
      blockCount: 1,
      blockEntityCount: 0,
      triangleCount: count,
      min: [0, 0, 0],
      max: [1, 1, 1],
      byteLength: count * 93,
      textures: [],
      parts,
    },
    bytes,
  }
}

test("packed pages preserve each source slice and align typed views without padding the payload", () => {
  const { metadata, bytes } = fixture()
  const layout = buildUploadLayout(metadata, true)
  const pages = [...layout.pages]
  assert.equal(pages.length, 1)
  const [page] = pages
  assert.equal(page.slices.length, 80)
  assert.equal(page.payloadBytes, 1488)
  assert.equal(page.byteLength, 1536)
  const wire = new Uint8Array(page.byteLength)
  for (const slice of page.slices) {
    wire.set(
      bytes.get(slice.bufferId).subarray(slice.offset, slice.offset + slice.length),
      slice.pageOffset,
    )
    if (slice.validation !== "none") assert.equal(slice.pageOffset % 4, 0)
  }
  for (const slice of page.slices) {
    assert.deepEqual(
      wire.subarray(slice.pageOffset, slice.pageOffset + slice.length),
      bytes.get(slice.bufferId).subarray(slice.offset, slice.offset + slice.length),
    )
  }
  assert.equal(wire.length - page.payloadBytes, 48)
})

test("arena offsets reconstruct independent local meshes without reordering transparent textures", () => {
  const { metadata, bytes } = fixture()
  const layout = buildUploadLayout(metadata, true)
  assert.equal(layout.arenas.length, 1)
  assert.deepEqual(layout.arenas[0].byteLengths, [576, 144, 384, 192, 192])
  assert.deepEqual(layout.parts[1].offsets, [36, 9, 24, 12, 12])
  const storage = layout.arenas[0].byteLengths.map((length) => new Uint8Array(length))
  for (const page of layout.pages) {
    for (const slice of page.slices) {
      const target = slice.target
      assert.equal(target.kind, "attribute")
      storage[target.attribute].set(
        bytes.get(slice.bufferId).subarray(slice.offset, slice.offset + slice.length),
        target.gpuOffset,
      )
    }
  }
  for (const [partIndex, part] of metadata.parts.entries()) {
    for (let attribute = 0; attribute < 5; attribute++) {
      const source = bytes.get(part.buffers[attribute])
      const offset = layout.parts[partIndex].offsets[attribute]
      assert.deepEqual(storage[attribute].subarray(offset, offset + source.length), source)
    }
    assert.deepEqual(
      new Uint32Array(storage[4].buffer, layout.parts[partIndex].offsets[4], 3),
      new Uint32Array([0, 1, 2]),
    )
  }
  assert.deepEqual(
    metadata.parts.slice(0, 3).map((part) => part.textureIndex),
    [0, 1, 0],
  )
  const serial = buildUploadLayout(metadata, false)
  assert.equal(serial.arenas.length, 16)
  assert.equal([...serial.pages].length, 80)
})

test("arena grouping admits exact boundary and an oversized part without splitting it", () => {
  const { metadata } = fixture(2)
  // Exact 16 MiB is unreachable with triangle-aligned indices; exercise the arithmetic boundary directly.
  metadata.parts[0].vertexCount = 419430
  metadata.parts[1].vertexCount = 201946
  metadata.parts[1].indexCount = 13
  const exact = buildUploadLayout(metadata, true)
  assert.equal(
    exact.arenas[0].byteLengths.reduce((a, b) => a + b, 0),
    ARENA_GROUP_BYTES,
  )
  assert.equal(exact.arenas.length, 1)
  metadata.parts.push({ ...metadata.parts[1], buffers: [...metadata.parts[1].buffers] })
  assert.equal(buildUploadLayout(metadata, true).arenas.length, 2)
  metadata.parts = metadata.parts.slice(0, 1)
  metadata.parts[0].vertexCount = 630000
  assert.equal(buildUploadLayout(metadata, true).arenas.length, 1)
  assert.ok(
    buildUploadLayout(metadata, true).arenas[0].byteLengths.reduce((a, b) => a + b, 0) >
      ARENA_GROUP_BYTES,
  )
  assert.ok([...buildUploadLayout(fixture().metadata, true).pages][0].byteLength <= UPLOAD_CHUNK)
})

test("packed texture pages keep batch-local indices and the 256-slice cap", () => {
  const { metadata } = fixture(52)
  metadata.textures = [
    { width: 1, height: 1, byteLength: 4, bufferId: 260, repeat: false },
    { width: 1, height: 1, byteLength: 4, bufferId: 261, repeat: false },
  ]
  const pages = [...buildUploadLayout(metadata, true).pages]
  assert.deepEqual(
    pages[0].slices.slice(0, 2).map((slice) => slice.target),
    [
      { kind: "texture", textureIndex: 0, x: 0, y: 0, width: 1, height: 1 },
      { kind: "texture", textureIndex: 1, x: 0, y: 0, width: 1, height: 1 },
    ],
  )
  assert.equal(pages[0].slices.length, 256)
  assert.equal(pages[1].slices.length, 6)
  assert.deepEqual(
    metadata.parts.slice(0, 3).map((part) => part.textureIndex),
    [0, 1, 0],
  )
})
