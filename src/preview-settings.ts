export type PreviewSettings = {
  memoryLimitEnabled: boolean
  memoryLimitMB: number
  chunkingEnabled: boolean
  chunkSize: number
  multithreadingEnabled: boolean
  threadCount: number
  conservativeMemoryScheduling: boolean
}

export const chunkSizes = [16, 32, 64, 128, 256]

export const defaultPreviewSettings: PreviewSettings = {
  memoryLimitEnabled: false,
  memoryLimitMB: 2048,
  chunkingEnabled: true,
  chunkSize: 64,
  multithreadingEnabled: true,
  threadCount: 4,
  conservativeMemoryScheduling: false,
}

export function savedPreviewSettings(): PreviewSettings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem("litematica-preview-settings") || "null")
    if (value === null || typeof value !== "object" || Array.isArray(value))
      return defaultPreviewSettings
    const settings = value as Record<string, unknown>
    return {
      memoryLimitEnabled:
        typeof settings.memoryLimitEnabled === "boolean"
          ? settings.memoryLimitEnabled
          : defaultPreviewSettings.memoryLimitEnabled,
      memoryLimitMB:
        typeof settings.memoryLimitMB === "number" &&
        Number.isInteger(settings.memoryLimitMB) &&
        settings.memoryLimitMB >= 2048 &&
        settings.memoryLimitMB <= 8192
          ? settings.memoryLimitMB
          : typeof settings.memoryLimitGiB === "number" &&
              Number.isInteger(settings.memoryLimitGiB) &&
              settings.memoryLimitGiB >= 2 &&
              settings.memoryLimitGiB <= 8
            ? settings.memoryLimitGiB * 1024
            : defaultPreviewSettings.memoryLimitMB,
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
      conservativeMemoryScheduling:
        typeof settings.conservativeMemoryScheduling === "boolean"
          ? settings.conservativeMemoryScheduling
          : typeof settings.speedFirst === "boolean"
            ? !settings.speedFirst
            : defaultPreviewSettings.conservativeMemoryScheduling,
    }
  } catch {
    return defaultPreviewSettings
  }
}
