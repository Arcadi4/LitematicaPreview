import assert from "node:assert/strict"
import { test } from "vite-plus/test"
import { PreviewStream } from "./preview-stream.ts"

const turn = () => new Promise((resolve) => setImmediate(resolve))

function batch(batchId, textureOffset, newTexture = batchId === 1) {
  const textures = newTexture
    ? [{ width: 1, height: 1, byteLength: 4, bufferId: 1, repeat: textureOffset !== 0 }]
    : []
  return {
    kind: "batch",
    batch: {
      batchId,
      textureOffset,
      metadata: {
        blockCount: 7,
        blockEntityCount: 2,
        triangleCount: 1,
        min: [batchId - 1, 0, 0],
        max: [batchId, 1, 1],
        geometryByteLength: 80,
        byteLength: 80 + textures.length * 4,
        textures,
        parts: [
          {
            vertexCount: 3,
            indexCount: 3,
            textureIndex: textureOffset + textures.length - 1,
            alphaMode: 0,
            vertexOffset: 0,
            indexOffset: 72,
            vertexStride: 24,
            indexSize: 2,
            origin: [batchId - 1, 0, 0],
            extent: [1, 1, 1],
          },
        ],
      },
    },
  }
}

function complete(events) {
  const metadata = {
    blockCount: 7,
    blockEntityCount: 2,
    triangleCount: 0,
    min: [Infinity, Infinity, Infinity],
    max: [-Infinity, -Infinity, -Infinity],
    byteLength: 0,
    geometryByteLength: 0,
    textures: [],
    parts: [],
  }
  let offset = 0
  let geometryOffset = 0
  for (const {
    batch: { metadata: source },
  } of events) {
    metadata.triangleCount += source.triangleCount
    metadata.byteLength += source.byteLength
    for (let axis = 0; axis < 3; axis++) {
      metadata.min[axis] = Math.min(metadata.min[axis], source.min[axis])
      metadata.max[axis] = Math.max(metadata.max[axis], source.max[axis])
    }
    for (const texture of source.textures)
      metadata.textures.push({ ...texture, bufferId: texture.bufferId + offset })
    for (const part of source.parts)
      metadata.parts.push({
        ...part,
        vertexOffset: part.vertexOffset + geometryOffset,
        indexOffset: part.indexOffset + geometryOffset,
      })
    offset += source.textures.length
    geometryOffset += source.geometryByteLength
    metadata.geometryByteLength += source.geometryByteLength
  }
  return { kind: "complete", metadata }
}

function sequence(events) {
  let index = 0
  return async () => {
    assert.ok(index < events.length, "no request follows the terminator")
    return events[index++]
  }
}

const guard = () => {}

test("consumes geometry before generation finishes and acknowledges only uploaded batches", async () => {
  const stream = new PreviewStream()
  const first = batch(1, 0)
  const second = batch(2, 1)
  const generated = Promise.withResolvers()
  const uploaded = Promise.withResolvers()
  const acknowledgements = []
  const uploads = []
  let generationFinished = false
  let committed = false
  const finished = stream
    .consume(
      async (previous) => {
        acknowledgements.push(previous)
        if (previous === null) return first
        if (previous === 1) return generated.promise
        return complete([first, second])
      },
      async (value) => {
        uploads.push(value.batchId)
        if (value.batchId === 1) {
          assert.equal(generationFinished, false)
          await uploaded.promise
        }
      },
      guard,
      4096,
    )
    .then((metadata) => {
      committed = true
      return metadata
    })
  await turn()
  assert.deepEqual(uploads, [1])
  assert.deepEqual(acknowledgements, [null])
  assert.equal(committed, false)
  // Native generation can finish while the first batch is still leased for upload.
  generationFinished = true
  generated.resolve(second)
  await turn()
  assert.deepEqual(acknowledgements, [null])
  uploaded.resolve()
  await finished
  assert.deepEqual(uploads, [1, 2])
  assert.deepEqual(acknowledgements, [null, 1, 2])
  assert.equal(committed, true)
})

test("deduplicated global textures survive local buffer ID reuse and publication-order remapping", async () => {
  const events = [batch(1, 0), batch(2, 1), batch(3, 1, true)]
  const uploadedTextures = []
  const boundTextures = []
  const stream = new PreviewStream()
  const metadata = await stream.consume(
    sequence([...events, complete(events)]),
    async (value) => {
      assert.equal(value.textureOffset, uploadedTextures.length)
      uploadedTextures.push(...value.metadata.textures)
      for (const part of value.metadata.parts)
        boundTextures.push(uploadedTextures[part.textureIndex])
    },
    guard,
    4096,
  )
  assert.equal(uploadedTextures.length, 2)
  assert.equal(boundTextures[0], boundTextures[1])
  assert.equal(boundTextures[2], uploadedTextures[1])
  assert.deepEqual(
    metadata.textures.map((texture) => texture.bufferId),
    [1, 2],
  )
  assert.deepEqual(
    metadata.parts.map((part) => part.vertexOffset),
    [0, 80, 160],
  )
})

test("late native failure never produces a completed model or another acknowledgement", async () => {
  const acknowledgements = []
  const uploads = []
  let completed = false
  const result = new PreviewStream().consume(
    async (previous) => {
      acknowledgements.push(previous)
      if (previous === null) return batch(1, 0)
      throw new Error("native chunk decode failed")
    },
    async (value) => {
      uploads.push(value.batchId)
    },
    guard,
    4096,
  )
  void result.then(
    () => {
      completed = true
    },
    () => {},
  )
  await assert.rejects(result, /native chunk decode failed/)
  assert.deepEqual(uploads, [1])
  assert.deepEqual(acknowledgements, [null, 1])
  assert.equal(completed, false)
})

test("failed upload leaves the native lease unacknowledged", async () => {
  const acknowledgements = []
  await assert.rejects(
    new PreviewStream().consume(
      async (previous) => {
        acknowledgements.push(previous)
        return batch(1, 0)
      },
      async () => {
        throw new Error("GPU upload failed")
      },
      guard,
      4096,
    ),
    /GPU upload failed/,
  )
  assert.deepEqual(acknowledgements, [null])
})

test("cancellation interrupts a pending next call and ignores its late data", async () => {
  const stream = new PreviewStream()
  const pending = Promise.withResolvers()
  const uploads = []
  const result = stream.consume(
    () => pending.promise,
    async (value) => {
      uploads.push(value.batchId)
    },
    guard,
    4096,
  )
  const rejected = assert.rejects(result, /Cancelled/)
  stream.cancel()
  await rejected
  pending.resolve(batch(1, 0))
  await turn()
  assert.deepEqual(uploads, [])
})

test("cancellation during upload prevents an acknowledgement or subsequent upload", async () => {
  const stream = new PreviewStream()
  const uploaded = Promise.withResolvers()
  const acknowledgements = []
  const result = stream.consume(
    async (previous) => {
      acknowledgements.push(previous)
      return batch(1, 0)
    },
    () => uploaded.promise,
    guard,
    4096,
  )
  await turn()
  const rejected = assert.rejects(result, /Cancelled/)
  stream.cancel()
  uploaded.resolve()
  await rejected
  assert.deepEqual(acknowledgements, [null])
})

test("generation guard prevents consuming a stale IPC response", async () => {
  let current = true
  const pending = Promise.withResolvers()
  const uploads = []
  const result = new PreviewStream().consume(
    () => pending.promise,
    async (value) => {
      uploads.push(value.batchId)
    },
    () => {
      if (!current) throw new Error("Cancelled")
    },
    4096,
  )
  current = false
  pending.resolve(batch(1, 0))
  await assert.rejects(result, /Cancelled/)
  assert.deepEqual(uploads, [])
})

const malformedBatches = [
  [
    "skipped ID",
    (value) => {
      value.batchId = 3
    },
  ],
  [
    "wrong texture offset",
    (value) => {
      value.textureOffset = 0
    },
  ],
  [
    "unpublished texture",
    (value) => {
      value.metadata.parts[0].textureIndex = 1
    },
  ],
  [
    "overlapping geometry",
    (value) => {
      value.metadata.parts[0].vertexOffset = 4
    },
  ],
  [
    "truncated payload",
    (value) => {
      value.metadata.byteLength--
    },
  ],
  [
    "incorrect triangle count",
    (value) => {
      value.metadata.triangleCount = 2
    },
  ],
  [
    "changed source count",
    (value) => {
      value.metadata.blockEntityCount++
    },
  ],
  [
    "reversed bounds",
    (value) => {
      value.metadata.min[0] = 9
    },
  ],
  [
    "fractional vertex count",
    (value) => {
      value.metadata.parts[0].vertexCount = 2.5
    },
  ],
  [
    "invalid alpha mode",
    (value) => {
      value.metadata.parts[0].alphaMode = 3
    },
  ],
  [
    "incomplete triangle",
    (value) => {
      value.metadata.parts[0].indexCount = 4
    },
  ],
  [
    "unsupported vertex layout",
    (value) => {
      value.metadata.parts[0].vertexStride = 25
    },
  ],
  [
    "invalid position transform",
    (value) => {
      value.metadata.parts[0].extent[0] = -1
    },
  ],
]
for (const [name, corrupt] of malformedBatches) {
  test(`rejects ${name} before uploading the malformed batch`, async () => {
    const events = [batch(1, 0), batch(2, 1)]
    corrupt(events[1].batch)
    const uploads = []
    await assert.rejects(
      new PreviewStream().consume(
        sequence(events),
        async (value) => {
          uploads.push(value.batchId)
        },
        guard,
        4096,
      ),
    )
    assert.deepEqual(uploads, [1])
  })
}

const malformedCompletions = [
  [
    "changed global bounds",
    (metadata) => {
      metadata.max[0]++
    },
  ],
  [
    "changed global counts",
    (metadata) => {
      metadata.blockCount++
    },
  ],
  [
    "changed texture descriptor",
    (metadata) => {
      metadata.textures[0].repeat = true
    },
  ],
  [
    "changed mesh descriptor",
    (metadata) => {
      metadata.parts[1].alphaMode = 2
    },
  ],
  [
    "reordered geometry",
    (metadata) => {
      ;[metadata.parts[0].vertexOffset, metadata.parts[1].vertexOffset] = [
        metadata.parts[1].vertexOffset,
        metadata.parts[0].vertexOffset,
      ]
    },
  ],
  [
    "missing batch",
    (metadata) => {
      metadata.parts.pop()
      metadata.byteLength -= 80
      metadata.geometryByteLength -= 80
      metadata.triangleCount--
    },
  ],
]
for (const [name, corrupt] of malformedCompletions) {
  test(`rejects completion with ${name} after staged uploads`, async () => {
    const events = [batch(1, 0), batch(2, 1)]
    const final = complete(events)
    corrupt(final.metadata)
    const uploads = []
    await assert.rejects(
      new PreviewStream().consume(
        sequence([...events, final]),
        async (value) => {
          uploads.push(value.batchId)
        },
        guard,
        4096,
      ),
    )
    assert.deepEqual(uploads, [1, 2])
  })
}
