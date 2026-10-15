import type { PreviewMetadata } from "./preview-stream"

export const UPLOAD_CHUNK = 1024 * 1024
export const MAX_READ_RANGES = 256
export const ARENA_GROUP_BYTES = 16 * 1024 * 1024

export type PreviewReadRange = { bufferId: number; offset: number; length: number }
export type UploadSlice = PreviewReadRange & {
  pageOffset: number
  target:
    | { kind: "texture"; textureIndex: number; x: number; y: number; width: number; height: number }
    | { kind: "geometry"; partIndex: number; index: boolean; gpuOffset: number }
}
export type UploadPage = { slices: UploadSlice[]; byteLength: number; payloadBytes: number }
export type MeshArena = {
  firstPart: number
  partCount: number
  byteLengths: [number, number]
}
export type MeshPartLayout = {
  arenaIndex: number
  offsets: [number, number]
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

export function buildUploadLayout(metadata: PreviewMetadata): UploadLayout {
  const arenas: MeshArena[] = []
  const parts: MeshPartLayout[] = []
  for (const [partIndex, part] of metadata.parts.entries()) {
    const lengths: MeshArena["byteLengths"] = [
      checked(part.vertexCount * part.vertexStride),
      align4(checked(part.indexCount * part.indexSize)),
    ]
    const partBytes = add(lengths[0], lengths[1])
    let arena = arenas[arenas.length - 1]
    if (!arena || add(arena.byteLengths[0] + arena.byteLengths[1], partBytes) > ARENA_GROUP_BYTES) {
      arena = { firstPart: partIndex, partCount: 0, byteLengths: [0, 0] }
      arenas.push(arena)
    }
    const offsets: MeshPartLayout["offsets"] = [...arena.byteLengths]
    arena.byteLengths[0] = add(arena.byteLengths[0], lengths[0])
    arena.byteLengths[1] = add(arena.byteLengths[1], lengths[1])
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
            target: { kind: "texture", textureIndex, x, y, width, height },
          }
        }
      }
    }
    for (const [partIndex, source] of metadata.parts.entries()) {
      const lengths = [
        source.vertexCount * source.vertexStride,
        align4(source.indexCount * source.indexSize),
      ]
      const offsets = [source.vertexOffset, source.indexOffset]
      for (let kind = 0; kind < 2; kind++) {
        for (let offset = 0; offset < lengths[kind]; offset += UPLOAD_CHUNK) {
          yield {
            bufferId: 0,
            offset: add(offsets[kind], offset),
            length: Math.min(UPLOAD_CHUNK, lengths[kind] - offset),
            pageOffset: 0,
            target: {
              kind: "geometry",
              partIndex,
              index: kind === 1,
              gpuOffset: add(parts[partIndex].offsets[kind], offset),
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
      let start = align4(page.byteLength)
      if (
        page.slices.length > 0 &&
        (add(start, slice.length) > UPLOAD_CHUNK || page.slices.length === MAX_READ_RANGES)
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
