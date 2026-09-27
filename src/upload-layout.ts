import { BUFFER_FORMATS, type PreviewMetadata } from "./preview-stream"

export const UPLOAD_CHUNK = 1024 * 1024
export const MAX_READ_RANGES = 256
export const ARENA_GROUP_BYTES = 16 * 1024 * 1024

export type PreviewReadRange = { bufferId: number; offset: number; length: number }
export type UploadSlice = PreviewReadRange & {
  pageOffset: number
  validation: "none" | "float32" | "uint32"
  vertexCount?: number
  target:
    | { kind: "texture"; textureIndex: number; x: number; y: number; width: number; height: number }
    | { kind: "attribute"; partIndex: number; attribute: 0 | 1 | 2 | 3 | 4; gpuOffset: number }
}
export type UploadPage = { slices: UploadSlice[]; byteLength: number; payloadBytes: number }
export type MeshArena = {
  firstPart: number
  partCount: number
  byteLengths: [number, number, number, number, number]
}
export type MeshPartLayout = {
  arenaIndex: number
  offsets: [number, number, number, number, number]
}
export type UploadLayout = {
  arenas: MeshArena[]
  parts: MeshPartLayout[]
  pages: Iterable<UploadPage>
}

const tooLarge = (): never => {
  throw new Error("The preview upload layout is too large.")
}

function checked(value: number): number {
  if (!Number.isSafeInteger(value) || value < 0) tooLarge()
  return value
}

function add(a: number, b: number): number {
  return checked(a + b)
}

const align4 = (value: number) => add(value, (4 - (value % 4)) % 4)

export function buildUploadLayout(metadata: PreviewMetadata, packed: boolean): UploadLayout {
  const arenas: MeshArena[] = []
  const parts: MeshPartLayout[] = []
  for (const [partIndex, part] of metadata.parts.entries()) {
    const lengths = BUFFER_FORMATS.map((format, attribute) =>
      checked(
        checked(attribute === 4 ? part.indexCount : part.vertexCount) *
          format.size *
          format.array.BYTES_PER_ELEMENT,
      ),
    ) as MeshArena["byteLengths"]
    const partBytes = lengths.reduce(add, 0)
    let arena = arenas[arenas.length - 1]
    if (
      !packed ||
      !arena ||
      (arena.partCount > 0 && add(arena.byteLengths.reduce(add, 0), partBytes) > ARENA_GROUP_BYTES)
    ) {
      arena = { firstPart: partIndex, partCount: 0, byteLengths: [0, 0, 0, 0, 0] }
      arenas.push(arena)
    }
    const offsets = [...arena.byteLengths] as MeshPartLayout["offsets"]
    for (let attribute = 0; attribute < 5; attribute++) {
      arena.byteLengths[attribute] = add(arena.byteLengths[attribute], lengths[attribute])
    }
    arena.partCount++
    parts.push({ arenaIndex: arenas.length - 1, offsets })
  }

  function* slices(): Generator<UploadSlice> {
    for (const [textureIndex, source] of metadata.textures.entries()) {
      const rowBytes = checked(source.width * 4)
      const tileWidth = Math.min(source.width, UPLOAD_CHUNK / 4)
      const rowsPerChunk = Math.max(1, Math.floor(UPLOAD_CHUNK / rowBytes))
      for (let y = 0; y < source.height; y += rowsPerChunk) {
        const height = Math.min(rowsPerChunk, source.height - y)
        for (let x = 0; x < source.width; x += tileWidth) {
          const width = Math.min(tileWidth, source.width - x)
          yield {
            bufferId: source.bufferId,
            offset: add(checked(y * rowBytes), checked(x * 4)),
            length: checked(width * height * 4),
            pageOffset: 0,
            validation: "none",
            target: { kind: "texture", textureIndex, x, y, width, height },
          }
        }
      }
    }
    for (const [partIndex, source] of metadata.parts.entries()) {
      for (let attribute = 0; attribute < 5; attribute++) {
        const format = BUFFER_FORMATS[attribute]
        const count = attribute === 4 ? source.indexCount : source.vertexCount
        const stride = format.size * format.array.BYTES_PER_ELEMENT
        const byteLength = checked(count * stride)
        const chunkSize = Math.floor(UPLOAD_CHUNK / stride) * stride
        for (let offset = 0; offset < byteLength; offset += chunkSize) {
          yield {
            bufferId: source.buffers[attribute],
            offset,
            length: Math.min(chunkSize, byteLength - offset),
            pageOffset: 0,
            validation:
              attribute === 4 ? "uint32" : attribute === 0 || attribute === 2 ? "float32" : "none",
            vertexCount: source.vertexCount,
            target: {
              kind: "attribute",
              partIndex,
              attribute: attribute as 0 | 1 | 2 | 3 | 4,
              gpuOffset: add(parts[partIndex].offsets[attribute], offset),
            },
          }
        }
      }
    }
  }

  function* pages(): Generator<UploadPage> {
    let page: UploadPage = { slices: [], byteLength: 0, payloadBytes: 0 }
    for (const slice of slices()) {
      checked(slice.length)
      if (slice.length === 0 || slice.length > UPLOAD_CHUNK) tooLarge()
      let start = packed ? align4(page.byteLength) : 0
      if (
        page.slices.length > 0 &&
        (!packed ||
          add(start, slice.length) > UPLOAD_CHUNK ||
          page.slices.length === MAX_READ_RANGES)
      ) {
        yield page
        page = { slices: [], byteLength: 0, payloadBytes: 0 }
        start = 0
      }
      slice.pageOffset = start
      page.slices.push(slice)
      page.byteLength = add(start, slice.length)
      page.payloadBytes = add(page.payloadBytes, slice.length)
    }
    if (page.slices.length) yield page
  }

  return { arenas, parts, pages: { [Symbol.iterator]: pages } }
}
