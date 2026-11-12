import { Badge, Button } from "@fluentui/react-components"
import { ArrowRight20Regular, Document20Regular, FolderOpen20Regular } from "@fluentui/react-icons"

import type { Bootstrap } from "./app-model"

export interface WelcomePanelProps {
  bootstrap: Bootstrap | null
  busy: boolean
  hidden: boolean
  onLoadDemo: (path: string) => void
  onOpenFile: () => void
}

export function WelcomePanel({
  bootstrap,
  busy,
  hidden,
  onLoadDemo,
  onOpenFile,
}: WelcomePanelProps) {
  return (
    <section
      className="bg-surface-secondary absolute inset-0 overflow-auto [overscroll-behavior:contain]"
      hidden={hidden}
    >
      <div className="mx-auto w-full max-w-5xl px-5 py-6 sm:px-8 sm:py-8 md:px-12 md:py-14">
        <div className="border-border bg-surface flex min-h-24 flex-wrap items-center gap-3 rounded-xl border border-dashed p-4 sm:flex-nowrap sm:gap-4 sm:p-5">
          <div className="text-accent bg-accent-soft flex h-10 w-10 shrink-0 items-center justify-center rounded-lg">
            <FolderOpen20Regular />
          </div>
          <div className="flex flex-1 flex-col gap-1">
            <strong className="text-sm font-semibold">Drop a schematic here</strong>
          </div>
          <Button
            appearance="primary"
            icon={<FolderOpen20Regular />}
            disabled={busy}
            onClick={onOpenFile}
            className="w-full sm:w-auto!"
          >
            {busy ? "Choosing file…" : "Open schematic"}
          </Button>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-2" aria-label="Supported formats">
          {bootstrap?.extensions.map((extension) => (
            <Badge key={extension} appearance="outline" shape="rounded">
              {extension}
            </Badge>
          ))}
        </div>
        {bootstrap && bootstrap.demos.length > 0 && (
          <section className="mt-8" aria-labelledby="demos-heading">
            <h2 id="demos-heading" className="m-0 mb-3 text-base font-semibold">
              Try a bundled example
            </h2>
            <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-3 lg:grid-cols-4">
              {bootstrap.demos.map((demo) => (
                <Button
                  key={demo.path}
                  appearance="outline"
                  className="border-border! bg-surface! hover:bg-surface-muted! hover:border-accent! flex! min-h-16! w-full! min-w-0! items-center! justify-start! gap-2.5! rounded-lg! p-3! text-left!"
                  onClick={() => onLoadDemo(demo.path)}
                  aria-label={`Open ${demo.name}, ${demo.extension}`}
                >
                  <span className="text-muted flex shrink-0">
                    <Document20Regular />
                  </span>
                  <span className="flex min-w-0 flex-1 flex-col gap-1">
                    <strong className="overflow-hidden text-xs font-semibold text-ellipsis whitespace-nowrap">
                      {demo.name}
                    </strong>
                    <span className="text-muted text-xs font-normal">{demo.extension}</span>
                  </span>
                  <ArrowRight20Regular className="text-muted w-4 shrink-0" />
                </Button>
              ))}
            </div>
          </section>
        )}
      </div>
    </section>
  )
}
