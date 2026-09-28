import { FolderOpen20Regular } from "@fluentui/react-icons"

export function DropOverlay() {
  return (
    <div
      className="absolute z-10 inset-3 sm:inset-4 grid place-items-center border-2 border-dashed border-accent rounded-xl bg-surface opacity-95 text-center pointer-events-none"
      role="status"
    >
      <div className="[&>svg]:w-10 [&>svg]:h-10 [&>svg]:mb-4 [&>svg]:text-accent">
        <FolderOpen20Regular />
        <h2 className="mt-0 mb-2 text-2xl font-semibold">Drop to preview</h2>
        <p className="mt-0 px-4 text-muted text-sm">The first supported schematic will open.</p>
      </div>
    </div>
  )
}
