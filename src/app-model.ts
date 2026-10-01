import type { PreviewMetadata } from "./renderer"

export type Bootstrap = {
  extensions: string[]
  demos: { name: string; path: string; extension: string }[]
  initialPath: string | null
  version: string
  requestId: number
  maxWorkerThreads: number
}
export type ThemePreference = "system" | "light" | "dark"
export type Loading = {
  path: string
  requestId: number
  phase: "decode" | "mesh" | "upload" | "stream"
  completed: number
  total: number
  uploadedBytes: number
}
export type MeshProgress = { requestId: number; phase: "mesh"; completed: number; total: number }
export type Loaded = { path: string; metadata: PreviewMetadata; seconds: number }
export type Notice = { intent: "error" | "success" | "info"; message: string }
export type DialogKind = "about" | "settings" | "error"
export type NativeCommand = "register_associations" | "unregister_associations" | "show_licenses"

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
