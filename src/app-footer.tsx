import { ShieldCheckmark20Regular } from "@fluentui/react-icons"
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
  choosing: boolean
  loaded: Loaded | null
  loading: Loading | null
  previewMemory: number | null
}

function phaseStatus(loading: Loading) {
  if (loading.phase === "decode") return "Decoding…"
  if (loading.phase === "mesh") return "Generating geometry…"
  if (loading.phase === "stream") return "Generating and uploading…"
  return "Uploading to graphics device…"
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

function IdleSummary({
  bootstrap,
  choosing,
  loading,
  previewMemory,
}: Omit<PreviewFooterProps, "loaded">) {
  return (
    <>
      <span role="status" className="flex items-center gap-2">
        {!loading && !choosing && <ShieldCheckmark20Regular className="shrink-0" />}
        {loading
          ? phaseStatus(loading)
          : choosing
            ? "Choose a schematic in the file dialog"
            : "Everything works offline."}
      </span>
      {loading && (
        <div className="ml-auto flex max-w-full flex-wrap justify-end gap-x-3 gap-y-1 text-right">
          {previewMemory === null ? (
            <span>Sampling memory…</span>
          ) : (
            <span title="Host and decoder private working sets (resident private pages only). WebView2 and GPU memory are excluded.">
              Process memory {formatMB(previewMemory)}
            </span>
          )}
          {(loading.phase === "upload" || loading.phase === "stream") && (
            <span>{formatMB(loading.uploadedBytes)} model data uploaded</span>
          )}
        </div>
      )}
      {!loading && bootstrap && <span className="ml-auto">v{bootstrap.version}</span>}
    </>
  )
}

export function PreviewFooter(props: PreviewFooterProps) {
  return (
    <footer
      className="flex-none flex items-center flex-wrap gap-x-4 sm:gap-x-6 gap-y-1.5 min-h-9 px-4 sm:px-6 py-2 border-t border-border bg-surface text-muted text-xs leading-normal [&_strong]:text-text [&_strong]:font-semibold"
      aria-label="Preview information"
    >
      {props.loaded ? <LoadedSummary loaded={props.loaded} /> : <IdleSummary {...props} />}
    </footer>
  )
}
