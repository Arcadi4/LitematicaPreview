export type TexturePack = { id: string; name: string; enabled: boolean }
/** All saved packs, highest priority first. Enabled packs layer over bundled vanilla resources. */
export type TexturePackState = { packs: TexturePack[] }
export type ImportedTexturePacks = {
  imported: number
  failures: string[]
  state: TexturePackState
}
export type Bootstrap = {
  extensions: string[]
  demos: { name: string; path: string; extension: string }[]
  initialPath: string | null
  version: string
  requestId: number
  maxWorkerThreads: number
  fileAssociationsSupported: boolean
  /** Replays a native viewport failure from before event listeners. */
  viewportError: string | null
  texturePacks: TexturePackState
  texturePackError: string | null
}
export type ThemePreference = "system" | "light" | "dark"
/** Completed preview statistics serialized by the native `preview-loaded` event. */
export type Metadata = {
  blockCount: number
  blockEntityCount: number
  triangleCount: number
  min: [number, number, number]
  max: [number, number, number]
  geometryByteLength: number
  byteLength: number
  textureCount: number
  partCount: number
}
export type Loading = {
  path: string
  requestId: number
  phase: "decode" | "mesh" | "upload"
  completed: number
  total: number
}
export type PreviewProgress = {
  requestId: number
  phase: "decode" | "mesh" | "upload"
  completed: number
  total: number
}
export type Loaded = { path: string; metadata: Metadata; seconds: number }
export type Notice = { intent: "error" | "success" | "info"; message: string }
export type DialogKind = "about" | "settings" | "error"
export type NativeCommand = "register_associations" | "unregister_associations" | "show_licenses"
/** Camera, grid and stage commands applied by the native viewport. */
export type ViewportCommand =
  | { kind: "orbit"; dx: number; dy: number }
  | { kind: "pan"; dx: number; dy: number }
  | { kind: "wheel"; deltaY: number; deltaMode: number }
  | { kind: "zoom"; factor: number }
  | { kind: "arrow"; dx: number; dy: number; panning: boolean }
  | { kind: "fit" }
  | { kind: "grid"; visible: boolean }
  /** Visible preview rectangle in CSS pixels; the native viewport clips geometry to it. */
  | { kind: "stage"; left: number; top: number; width: number; height: number }
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
