import {
  dimensions,
  formatMB,
  numbers,
  type Bootstrap,
  type Loaded,
  type Loading,
} from "./app-model"

export interface PreviewFooterProps {
  bootstrap: Bootstrap | null
  loaded: Loaded | null
  loading: Loading | null
  previewMemory: number | null
}

function phaseStatus(loading: Loading) {
  if (loading.phase === "decode") return "Decoding…"
  if (loading.phase === "upload") return "Finishing GPU uploads…"
  return "Generating geometry…"
}

function LoadedSummary({ loaded }: { loaded: Loaded }) {
  const size = loaded.metadata.max
    .map((maximum, axis) => dimensions.format(maximum - loaded.metadata.min[axis]))
    .join(" × ")
  return (
    <>
      <span>
        <strong>{numbers.format(loaded.metadata.blockCount)}</strong> blocks
      </span>
      <span title="Geometry dimensions in blocks">{size} blocks</span>
      <span>{numbers.format(loaded.metadata.triangleCount)} triangles</span>
      <span className="ml-auto">
        {formatMB(loaded.metadata.byteLength)} model data loaded in {loaded.seconds.toFixed(2)} s.
      </span>
    </>
  )
}

function IdleSummary({ bootstrap, loading, previewMemory }: Omit<PreviewFooterProps, "loaded">) {
  return (
    <>
      {loading && (
        <>
          <span role="status">{phaseStatus(loading)}</span>
          <div className="ml-auto flex max-w-full flex-wrap justify-end gap-x-3 gap-y-1 text-right">
            {previewMemory === null ? (
              <span>Sampling memory…</span>
            ) : (
              <span title="Host process private working set (resident private pages only). WebView2 and GPU memory are excluded.">
                Host memory {formatMB(previewMemory)}
              </span>
            )}
          </div>
        </>
      )}
      {!loading && bootstrap && <span className="ml-auto">v{bootstrap.version}</span>}
    </>
  )
}

export function PreviewFooter(props: PreviewFooterProps) {
  return (
    <footer
      className="border-border bg-surface text-muted [&_strong]:text-text flex min-h-9 flex-none flex-wrap items-center gap-x-4 gap-y-1.5 border-t px-4 py-2 text-xs leading-normal sm:gap-x-6 sm:px-6 [&_strong]:font-semibold"
      aria-label="Preview information"
    >
      {props.loaded ? <LoadedSummary loaded={props.loaded} /> : <IdleSummary {...props} />}
    </footer>
  )
}
