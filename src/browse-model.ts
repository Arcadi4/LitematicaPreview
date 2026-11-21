export type BrowseSelection = { paths: string[]; index: number }

export function parseBrowseSelection(value: unknown): BrowseSelection | null {
  if (typeof value !== "object" || value === null) return null
  const { paths, index } = value as { paths?: unknown; index?: unknown }
  if (!Array.isArray(paths) || paths.length === 0) return null
  if (!paths.every((path) => typeof path === "string" && path.length > 0)) return null
  if (typeof index !== "number" || !Number.isInteger(index) || index < 0 || index >= paths.length)
    return null
  return { paths: paths as string[], index }
}
