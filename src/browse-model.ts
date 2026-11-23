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

export type BrowseSource = { label: string; title: string }

function parentOf(path: string) {
  const end = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"))
  return end < 0 ? "" : path.slice(0, end)
}

/** Names the shared folder, or counts the files when they come from several folders. */
export function browseSource(paths: readonly string[]): BrowseSource {
  const parent = parentOf(paths[0] ?? "")
  const shared = paths.every((path) => parentOf(path).toLowerCase() === parent.toLowerCase())
  if (shared && parent) {
    const name = parent.slice(Math.max(parent.lastIndexOf("/"), parent.lastIndexOf("\\")) + 1)
    return { label: name || parent, title: parent }
  }
  return { label: `${paths.length} selected files`, title: paths.join("\n") }
}
