import { FolderOpen20Regular } from "@fluentui/react-icons"

export function DropOverlay() {
  return (
    <div
      className="border-accent bg-surface pointer-events-none absolute inset-3 z-10 grid place-items-center rounded-xl border-2 border-dashed text-center opacity-95 sm:inset-4"
      role="status"
    >
      <div className="[&>svg]:text-accent [&>svg]:mb-4 [&>svg]:h-10 [&>svg]:w-10">
        <FolderOpen20Regular />
        <h2 className="mt-0 mb-2 text-2xl font-semibold">Drop to preview</h2>
      </div>
    </div>
  )
}
