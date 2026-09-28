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
      className="absolute inset-0 overflow-auto [overscroll-behavior:contain]"
      hidden={hidden}
    >
      <div className="w-full max-w-5xl mx-auto px-5 py-6 sm:px-8 sm:py-8 md:px-12 md:py-14">
        <div className="flex items-center gap-3 sm:gap-4 min-h-24 p-4 sm:p-5 border border-dashed border-border rounded-xl bg-surface flex-wrap sm:flex-nowrap">
          <div className="flex justify-center items-center w-10 h-10 text-accent bg-accent-soft rounded-lg shrink-0">
            <FolderOpen20Regular />
          </div>
          <div className="flex flex-1 flex-col gap-1">
            <strong className="text-sm font-semibold">Drop a schematic here</strong>
            <span className="text-muted text-xs">or choose a file from your computer</span>
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
        <div className="flex items-center flex-wrap gap-2 mt-4" aria-label="Supported formats">
          {bootstrap ? (
            bootstrap.extensions.map((extension) => (
              <Badge key={extension} appearance="outline" shape="rounded">
                {extension}
              </Badge>
            ))
          ) : (
            <span className="font-normal text-muted" role="status">
              Preparing the schematic viewer…
            </span>
          )}
        </div>
        {bootstrap && bootstrap.demos.length > 0 && (
          <section className="mt-8" aria-labelledby="demos-heading">
            <div className="flex items-baseline justify-between flex-wrap gap-x-5 gap-y-1.5 mb-3">
              <h2 id="demos-heading" className="m-0 text-base font-semibold">
                Try a bundled example
              </h2>
              <span className="text-muted text-xs">Explore a format, no download required</span>
            </div>
            <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-2.5">
              {bootstrap.demos.map((demo) => (
                <Button
                  key={demo.path}
                  appearance="outline"
                  className="flex! justify-start! items-center! gap-2.5! w-full! min-w-0! min-h-16! p-3! border-border! rounded-lg! bg-surface! text-left! hover:bg-surface-muted! hover:border-accent!"
                  onClick={() => onLoadDemo(demo.path)}
                  aria-label={`Open ${demo.name}, ${demo.extension}`}
                >
                  <span className="text-muted flex shrink-0">
                    <Document20Regular />
                  </span>
                  <span className="flex flex-1 min-w-0 flex-col gap-1">
                    <strong className="text-xs font-semibold overflow-hidden text-ellipsis whitespace-nowrap">
                      {demo.name}
                    </strong>
                    <span className="text-muted text-xs font-normal">{demo.extension}</span>
                  </span>
                  <ArrowRight20Regular className="text-muted shrink-0 w-4" />
                </Button>
              ))}
            </div>
          </section>
        )}
      </div>
    </section>
  )
}
