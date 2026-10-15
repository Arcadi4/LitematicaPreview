import assert from "node:assert/strict"
import { test } from "vite-plus/test"
import {
  ARENA_GROUP_BYTES,
  buildUploadLayout,
  MAX_READ_RANGES,
  UPLOAD_CHUNK,
} from "./upload-layout.ts"

function fixture(count = 16) {
  let cursor = 0
  const parts = Array.from({ length: count }, (_, index) => {
    const vertexStride = index % 2 ? 28 : 24
    const indexSize = index % 2 ? 4 : 2
    const vertexOffset = cursor
    const indexOffset = vertexOffset + 3 * vertexStride
    cursor = Math.ceil((indexOffset + 3 * indexSize) / 4) * 4
    return {
      vertexCount: 3,
      indexCount: 3,
      vertexOffset,
      indexOffset,
      vertexStride,
      indexSize,
      textureIndex: 0,
      alphaMode: 0,
      origin: [0, 0, 0],
      extent: [1, 1, 1],
    }
  })
  const geometry = Uint8Array.from({ length: cursor }, (_, index) => (index * 17) % 251)
  for (const part of parts) {
    const view = new DataView(geometry.buffer)
    for (let index = 0; index < 3; index++) {
      if (part.indexSize === 2) view.setUint16(part.indexOffset + index * 2, index, true)
      else view.setUint32(part.indexOffset + index * 4, index, true)
    }
  }
  return {
    metadata: {
      blockCount: 1,
      blockEntityCount: 0,
      triangleCount: count,
      min: [0, 0, 0],
      max: [1, 1, 1],
      geometryByteLength: cursor,
      byteLength: cursor,
      textures: [],
      parts,
    },
    bytes: new Map([[0, geometry]]),
  }
}

test("packed pages reconstruct mixed vertex and index layouts without converting bytes", () => {
  const { metadata, bytes } = fixture()
  const layout = buildUploadLayout(metadata)
  const storage = layout.arenas.map((arena) =>
    arena.byteLengths.map((length) => new Uint8Array(length)),
  )
  let uploaded = 0
  for (const page of layout.pages) {
    assert.ok(page.byteLength <= UPLOAD_CHUNK)
    assert.ok(page.slices.length <= MAX_READ_RANGES)
    const wire = new Uint8Array(page.byteLength)
    for (const slice of page.slices) {
      assert.equal(slice.pageOffset % 4, 0)
      wire.set(
        bytes.get(slice.bufferId).subarray(slice.offset, slice.offset + slice.length),
        slice.pageOffset,
      )
    }
    for (const slice of page.slices) {
      const target = slice.target
      const arena = layout.parts[target.partIndex].arenaIndex
      storage[arena][target.index ? 1 : 0].set(
        wire.subarray(slice.pageOffset, slice.pageOffset + slice.length),
        target.gpuOffset,
      )
      uploaded += slice.length
    }
  }
  assert.equal(uploaded, metadata.geometryByteLength)
  for (const [index, part] of metadata.parts.entries()) {
    const target = layout.parts[index]
    const [vertices, indices] = storage[target.arenaIndex]
    const vertexBytes = part.vertexCount * part.vertexStride
    const indexBytes = part.indexCount * part.indexSize
    assert.deepEqual(
      vertices.subarray(target.offsets[0], target.offsets[0] + vertexBytes),
      bytes.get(0).subarray(part.vertexOffset, part.indexOffset),
    )
    assert.deepEqual(
      indices.subarray(target.offsets[1], target.offsets[1] + indexBytes),
      bytes.get(0).subarray(part.indexOffset, part.indexOffset + indexBytes),
    )
    const view = new DataView(indices.buffer, target.offsets[1], indexBytes)
    assert.deepEqual(
      Array.from({ length: 3 }, (_, i) =>
        part.indexSize === 2 ? view.getUint16(i * 2, true) : view.getUint32(i * 4, true),
      ),
      [0, 1, 2],
    )
  }
})

test("GPU arenas retain an exact-capacity part and start a new arena for the next draw", () => {
  const { metadata } = fixture(2)
  const first = metadata.parts[0]
  Object.assign(first, { vertexStride: 28, indexSize: 4, vertexCount: 599185, indexCount: 9 })
  first.indexOffset = first.vertexCount * first.vertexStride
  const end = first.indexOffset + first.indexCount * first.indexSize
  assert.equal(end, ARENA_GROUP_BYTES)
  const second = metadata.parts[1]
  second.vertexOffset = end
  second.indexOffset = end + second.vertexCount * second.vertexStride
  const layout = buildUploadLayout(metadata)
  assert.equal(layout.parts[0].arenaIndex, 0)
  assert.equal(layout.parts[1].arenaIndex, 1)
  assert.deepEqual(layout.parts[1].offsets, [0, 0])
  first.vertexCount++
  metadata.parts = [first]
  const oversized = buildUploadLayout(metadata)
  assert.equal(oversized.parts.length, 1)
  assert.ok(
    oversized.arenas[0].byteLengths.reduce((sum, length) => sum + length, 0) > ARENA_GROUP_BYTES,
  )
  for (const page of oversized.pages) assert.ok(page.byteLength <= UPLOAD_CHUNK)
})

test("texture rows and many tiny draws remain bounded and preserve all texture pixels", () => {
  const { metadata, bytes } = fixture(130)
  const width = 512
  const height = 1025
  const pixels = Uint8Array.from({ length: width * height * 4 }, (_, index) => index % 251)
  metadata.textures = [{ width, height, byteLength: pixels.length, bufferId: 1, repeat: true }]
  bytes.set(1, pixels)
  metadata.byteLength += pixels.length
  const restored = new Uint8Array(pixels.length)
  for (const page of buildUploadLayout(metadata).pages) {
    assert.ok(page.byteLength <= UPLOAD_CHUNK)
    assert.ok(page.slices.length <= MAX_READ_RANGES)
    for (const slice of page.slices) {
      if (slice.target.kind !== "texture") continue
      const { x, y, width: tileWidth, height: tileHeight } = slice.target
      const data = bytes.get(slice.bufferId).subarray(slice.offset, slice.offset + slice.length)
      for (let row = 0; row < tileHeight; row++) {
        restored.set(
          data.subarray(row * tileWidth * 4, (row + 1) * tileWidth * 4),
          ((y + row) * width + x) * 4,
        )
      }
    }
  }
  assert.deepEqual(restored, pixels)
})
