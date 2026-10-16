export type Bootstrap = {
  extensions: string[]
  demos: { name: string; path: string; extension: string }[]
  initialPath: string | null
  version: string
  requestId: number
  maxWorkerThreads: number
}
export type ThemePreference = "system" | "light" | "dark"
/**
 * The completed preview descriptors, exactly as the native `preview-loaded`
 * event serializes the protocol metadata (camelCase field names).
 */
export type Metadata = {
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
export type Loading = {
  path: string
  requestId: number
  phase: "decode" | "mesh" | "stream"
  completed: number
  total: number
}
export type PreviewProgress = {
  requestId: number
  phase: "decode" | "mesh" | "stream"
  completed: number
  total: number
}
export type Loaded = { path: string; metadata: Metadata; seconds: number }
export type Notice = { intent: "error" | "success" | "info"; message: string }
export type DialogKind = "about" | "settings" | "error"
export type NativeCommand = "register_associations" | "unregister_associations" | "show_licenses"
/** Camera and grid commands the native viewport applies on the next frame. */
export type ViewportCommand =
  | { kind: "orbit"; dx: number; dy: number }
  | { kind: "pan"; dx: number; dy: number }
  | { kind: "wheel"; deltaY: number; deltaMode: number }
  | { kind: "zoom"; factor: number }
  | { kind: "arrow"; dx: number; dy: number; panning: boolean }
  | { kind: "fit" }
  | { kind: "grid"; visible: boolean }
export type PreviewLoaded = { requestId: number; metadata: Metadata }
export type PreviewFailed = { requestId: number; message: string }
export type ViewportFailed = { message: string }

export const appName = "Litematica Preview"
export const numbers = new Intl.NumberFormat()
export const dimensions = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 2,
})
const megabytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 })
export const formatMB = (bytes: number) => `${megabytes.format(bytes / (1024 * 1024))} MB`
export const fileName = (path: string) => path.split(/[\\/]/).pop() || path
export const errorMessage = (error: unknown) =>
  error instanceof Error ? error.message : String(error)
export const isTheme = (value: unknown): value is ThemePreference =>
  value === "system" || value === "light" || value === "dark"

export function savedTheme(): ThemePreference {
  try {
    const value = localStorage.getItem("litematica-preview-theme")
    return isTheme(value) ? value : "system"
  } catch {
    return "system"
  }
}
