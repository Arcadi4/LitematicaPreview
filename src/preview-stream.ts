export type PreviewMetadata = {
  blockCount: number
  blockEntityCount: number
  triangleCount: number
  min: [number, number, number]
  max: [number, number, number]
  geometryByteLength: number
  byteLength: number
  textures: {
    width: number
    height: number
    byteLength: number
    bufferId: number
    repeat: boolean
  }[]
  parts: {
    vertexCount: number
    indexCount: number
    textureIndex: number
    alphaMode: 0 | 1 | 2
    vertexOffset: number
    indexOffset: number
    vertexStride: 24 | 28
    indexSize: 2 | 4
    origin: [number, number, number]
    extent: [number, number, number]
  }[]
}

export type PreviewBatch = {
  batchId: number
  textureOffset: number
  metadata: PreviewMetadata
}

export type PreviewStreamEvent =
  | { kind: "batch"; batch: PreviewBatch }
  | { kind: "complete"; metadata: PreviewMetadata }

function integer(
  value: unknown,
  name: string,
  maximum = Number.MAX_SAFE_INTEGER,
  minimum = 0,
): number {
  if (
    typeof value !== "number" ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > maximum
  ) {
    throw new Error(`Invalid preview ${name}.`)
  }
  return value
}

function record(value: unknown, name: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error(`Invalid preview ${name}.`)
  return value as Record<string, unknown>
}

export function validateMetadata(
  metadata: PreviewMetadata,
  maxTextureSize: number,
  textureOffset = 0,
): void {
  const root = record(metadata, "metadata")
  const byteLength = integer(root.byteLength, "payload byte length", Number.MAX_SAFE_INTEGER, 1)
  const geometryBytes = integer(root.geometryByteLength, "geometry byte length", byteLength, 1)
  integer(root.blockCount, "block count", Number.MAX_SAFE_INTEGER, 1)
  integer(root.blockEntityCount, "block entity count")
  integer(root.triangleCount, "triangle count", Number.MAX_SAFE_INTEGER, 1)
  for (const bound of [root.min, root.max]) {
    if (
      !Array.isArray(bound) ||
      bound.length !== 3 ||
      bound.some((v: unknown) => typeof v !== "number" || !Number.isFinite(v))
    ) {
      throw new Error("The preview geometry bounds are invalid.")
    }
  }
  const min = root.min as number[]
  const max = root.max as number[]
  for (let axis = 0; axis < 3; axis++) {
    if (min[axis] > max[axis]) throw new Error("The preview geometry bounds are reversed.")
  }
  if (
    !Array.isArray(root.textures) ||
    root.textures.length + textureOffset === 0 ||
    !Array.isArray(root.parts) ||
    root.parts.length === 0
  ) {
    throw new Error("The preview must contain textures and renderable mesh parts.")
  }
  const bufferCount = integer(root.textures.length + 1, "buffer count", 0xffffffff, 1)
  const bufferIds = new Set<number>([0])
  let totalBytes = geometryBytes
  const consume = (id: unknown, bytes: number) => {
    const bufferId = integer(id, "buffer ID", bufferCount - 1)
    if (bufferIds.has(bufferId)) throw new Error("The preview contains a duplicate buffer ID.")
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > byteLength - totalBytes) {
      throw new Error("The preview contains truncated or oversized geometry.")
    }
    bufferIds.add(bufferId)
    totalBytes += bytes
  }
  for (const value of root.textures) {
    const texture = record(value, "texture")
    const width = integer(texture.width, "texture width", 0x7fffffff, 1)
    const height = integer(texture.height, "texture height", 0x7fffffff, 1)
    if (width > maxTextureSize || height > maxTextureSize) {
      throw new Error(
        `A block texture exceeds the graphics device's ${maxTextureSize}-pixel texture limit.`,
      )
    }
    const bytes = integer(texture.byteLength, "texture byte length")
    if (bytes !== width * height * 4)
      throw new Error("A block texture has an invalid RGBA byte length.")
    if (typeof texture.repeat !== "boolean")
      throw new Error("A block texture has an invalid repeat mode.")
    consume(texture.bufferId, bytes)
  }
  let triangles = 0
  let geometryOffset = 0
  for (const value of root.parts) {
    const part = record(value, "mesh part")
    const vertices = integer(part.vertexCount, "vertex count", 0xffffffff, 1)
    const indices = integer(part.indexCount, "index count", 0x7fffffff, 1)
    integer(part.textureIndex, "texture index", root.textures.length + textureOffset - 1)
    integer(part.alphaMode, "alpha mode", 2)
    if (indices % 3 !== 0) throw new Error("A preview mesh contains an incomplete triangle.")
    const stride = integer(part.vertexStride, "vertex stride")
    const indexSize = integer(part.indexSize, "index size")
    if ((stride !== 24 && stride !== 28) || (indexSize !== 2 && indexSize !== 4))
      throw new Error("A preview mesh has an invalid packed layout.")
    if (indexSize === 2 && vertices > 65536)
      throw new Error("A preview mesh has an invalid index width.")
    const vertexOffset = integer(part.vertexOffset, "vertex offset", geometryBytes)
    const indexOffset = integer(part.indexOffset, "index offset", geometryBytes)
    const end = indexOffset + indices * indexSize
    if (
      vertexOffset !== geometryOffset ||
      indexOffset !== vertexOffset + vertices * stride ||
      !Number.isSafeInteger(end) ||
      end > geometryBytes
    )
      throw new Error("The preview contains truncated or overlapping geometry.")
    geometryOffset = Math.ceil(end / 4) * 4
    for (const [name, bound] of [
      ["origin", part.origin],
      ["extent", part.extent],
    ] as const) {
      if (
        !Array.isArray(bound) ||
        bound.length !== 3 ||
        bound.some(
          (value: unknown) =>
            typeof value !== "number" ||
            !Number.isFinite(value) ||
            (name === "extent" && value < 0),
        )
      )
        throw new Error("A preview mesh has an invalid position transform.")
    }
    triangles = integer(triangles + indices / 3, "accumulated triangle count")
  }
  if (triangles !== root.triangleCount)
    throw new Error("The preview triangle count does not match its mesh parts.")
  if (totalBytes !== byteLength) throw new Error("The preview has an unexpected payload length.")
  if (geometryOffset !== geometryBytes)
    throw new Error("The preview has an unexpected geometry length.")
}

// Retain descriptors only, never batch payloads or copies of earlier descriptor arrays.
// Native buffer IDs are local within a batch and globally remapped at completion.
export class PreviewStream {
  private cancelWait: (() => void) | null = null
  private stopped = false

  cancel(): void {
    this.stopped = true
    this.cancelWait?.()
    this.cancelWait = null
  }

  private wait(next: Promise<PreviewStreamEvent>): Promise<PreviewStreamEvent> {
    return new Promise((resolve, reject) => {
      if (this.stopped) {
        reject(new Error("Cancelled"))
        void next.catch(() => {})
        return
      }
      const cancel = () => reject(new Error("Cancelled"))
      this.cancelWait = cancel
      void next.then(resolve, reject).finally(() => {
        if (this.cancelWait === cancel) this.cancelWait = null
      })
    })
  }

  async consume(
    next: (previousBatchId: number | null) => Promise<PreviewStreamEvent>,
    upload: (batch: PreviewBatch) => Promise<void>,
    guard: () => void,
    maxTextureSize: number,
  ): Promise<PreviewMetadata> {
    const check = () => {
      if (this.stopped) throw new Error("Cancelled")
      guard()
    }
    const batches: PreviewMetadata[] = []
    let previous: number | null = null
    let textureCount = 0
    let partCount = 0
    let byteLength = 0
    let geometryByteLength = 0
    let triangleCount = 0
    const minimum = [Infinity, Infinity, Infinity]
    const maximum = [-Infinity, -Infinity, -Infinity]
    while (true) {
      check()
      // Cancel a wait immediately even if IPC is still unwinding its native release.
      const event = await this.wait(next(previous))
      check()
      record(event, "stream event")
      if (event.kind === "complete") {
        const metadata = event.metadata
        if (!metadata || batches.length === 0)
          throw new Error("The preview stream completed without geometry.")
        validateMetadata(metadata, maxTextureSize)
        const first = batches[0]
        if (
          metadata.blockCount !== first.blockCount ||
          metadata.blockEntityCount !== first.blockEntityCount ||
          metadata.textures.length !== textureCount ||
          metadata.parts.length !== partCount ||
          metadata.byteLength !== byteLength ||
          metadata.geometryByteLength !== geometryByteLength ||
          metadata.triangleCount !== triangleCount ||
          metadata.min.some((bound, axis) => bound !== minimum[axis]) ||
          metadata.max.some((bound, axis) => bound !== maximum[axis])
        )
          throw new Error("The preview completion summary does not match its batches.")
        let textureIndex = 0
        let partIndex = 0
        let bufferOffset = 0
        let geometryOffset = 0
        for (const batch of batches) {
          for (const source of batch.textures) {
            const final = metadata.textures[textureIndex++]
            if (
              final.width !== source.width ||
              final.height !== source.height ||
              final.byteLength !== source.byteLength ||
              final.repeat !== source.repeat ||
              final.bufferId !== bufferOffset + source.bufferId
            )
              throw new Error("The preview completion texture does not match its batch.")
          }
          for (const source of batch.parts) {
            const final = metadata.parts[partIndex++]
            if (
              final.vertexCount !== source.vertexCount ||
              final.indexCount !== source.indexCount ||
              final.textureIndex !== source.textureIndex ||
              final.alphaMode !== source.alphaMode ||
              final.vertexOffset !== geometryOffset + source.vertexOffset ||
              final.indexOffset !== geometryOffset + source.indexOffset ||
              final.vertexStride !== source.vertexStride ||
              final.indexSize !== source.indexSize ||
              final.origin.some((value, axis) => value !== source.origin[axis]) ||
              final.extent.some((value, axis) => value !== source.extent[axis])
            )
              throw new Error("The preview completion mesh does not match its batch.")
          }
          bufferOffset += batch.textures.length
          geometryOffset += batch.geometryByteLength
        }
        check()
        return metadata
      }
      if (event.kind !== "batch") throw new Error("Invalid preview stream event kind.")
      const batch = record(event.batch, "batch")
      const batchId = integer(batch.batchId, "batch ID", Number.MAX_SAFE_INTEGER, 1)
      if (batchId !== (previous ?? 0) + 1)
        throw new Error("The preview batch order is inconsistent.")
      if (integer(batch.textureOffset, "texture offset") !== textureCount)
        throw new Error("The preview batch texture offset is inconsistent.")
      const metadata = batch.metadata as PreviewMetadata
      validateMetadata(metadata, maxTextureSize, textureCount)
      if (
        batches.length &&
        (metadata.blockCount !== batches[0].blockCount ||
          metadata.blockEntityCount !== batches[0].blockEntityCount)
      )
        throw new Error("The preview batch source counts are inconsistent.")
      textureCount = integer(
        textureCount + metadata.textures.length,
        "accumulated texture count",
        0xffffffff,
      )
      partCount = integer(partCount + metadata.parts.length, "accumulated part count")
      byteLength = integer(byteLength + metadata.byteLength, "accumulated byte length")
      geometryByteLength = integer(
        geometryByteLength + metadata.geometryByteLength,
        "accumulated geometry length",
      )
      triangleCount = integer(triangleCount + metadata.triangleCount, "accumulated triangle count")
      for (let axis = 0; axis < 3; axis++) {
        minimum[axis] = Math.min(minimum[axis], metadata.min[axis])
        maximum[axis] = Math.max(maximum[axis], metadata.max[axis])
      }
      check()
      await upload(event.batch)
      check()
      batches.push(metadata)
      // The next call acknowledges this batch only after every upload/read has drained.
      previous = batchId
    }
  }
}
