export type PreviewSettings = {
  chunkingEnabled: boolean
  chunkSize: number
  multithreadingEnabled: boolean
  threadCount: number
}

export const chunkSizes = [16, 32, 64, 128, 256]

export const defaultPreviewSettings: PreviewSettings = {
  chunkingEnabled: true,
  chunkSize: 64,
  multithreadingEnabled: true,
  threadCount: 4,
}

export function savedPreviewSettings(): PreviewSettings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem("litematica-preview-settings") || "null")
    if (value === null || typeof value !== "object" || Array.isArray(value))
      return defaultPreviewSettings
    const settings = value as Record<string, unknown>
    return {
      chunkingEnabled:
        typeof settings.chunkingEnabled === "boolean"
          ? settings.chunkingEnabled
          : defaultPreviewSettings.chunkingEnabled,
      chunkSize:
        typeof settings.chunkSize === "number" && chunkSizes.includes(settings.chunkSize)
          ? settings.chunkSize
          : defaultPreviewSettings.chunkSize,
      multithreadingEnabled:
        settings.multithreadingEnabled === true && settings.chunkingEnabled !== false,
      threadCount:
        typeof settings.threadCount === "number" &&
        Number.isInteger(settings.threadCount) &&
        settings.threadCount >= 2 &&
        settings.threadCount <= 8
          ? settings.threadCount
          : defaultPreviewSettings.threadCount,
    }
  } catch {
    return defaultPreviewSettings
  }
}
