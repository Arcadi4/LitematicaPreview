import type { RefObject } from "react"
import { Button, ProgressBar } from "@fluentui/react-components"
import { Cube24Regular } from "@fluentui/react-icons"
import { fileName, formatMB, numbers, type Loaded, type Loading } from "./app-model"

export interface PreviewStageProps {
  canvasRef: RefObject<HTMLCanvasElement | null>
  loaded: Loaded | null
  loading: Loading | null
  onCancel: () => void
  stageActive: boolean
}

function percent(completed: number, total: number) {
  return Math.round((completed / total) * 100)
}

function progressSummary(loading: Loading) {
  if (loading.phase === "decode") return "Reading blocks locally."
  if (loading.phase === "mesh" || loading.phase === "stream") {
    if (loading.total <= 0) return "Generating geometry and uploading ready batches."
    return `Generating geometry: ${numbers.format(loading.completed)} / ${numbers.format(loading.total)} chunks (${percent(loading.completed, loading.total)}%).`
  }
  return `Uploading geometry and textures: ${formatMB(loading.completed)} / ${formatMB(loading.total)} (${percent(loading.completed, loading.total)}%).`
}

function progressLabel(loading: Loading) {
  if (loading.phase === "decode") return "Reading blocks"
  if (loading.phase === "mesh" || loading.phase === "stream") return "Generating geometry"
  return "Uploading model data"
}

function LoadingOverlay({ loading, onCancel }: { loading: Loading; onCancel: () => void }) {
  const indeterminate = loading.phase === "decode" || loading.total === 0
  return (
    <div className="absolute inset-0 grid place-items-center bg-surface-secondary p-6 sm:p-8">
      <div
        className="max-w-md w-full p-6 sm:p-8 border border-border rounded-xl bg-surface text-center shadow-lg"
        role="status"
        aria-live="polite"
      >
        <div className="inline-flex justify-center items-center w-12 h-12 mb-4 rounded-xl bg-accent-soft text-accent">
          <Cube24Regular />
        </div>
        <h2 className="mt-0 mb-2 text-xl font-semibold leading-snug">
          {loading.phase === "decode" ? "Preparing your schematic" : "Building the preview"}
        </h2>
        <p
          className="mt-0 mb-3 text-sm leading-relaxed break-words font-semibold"
          title={loading.path}
        >
          {fileName(loading.path)}
        </p>
        <p className="mt-0 mb-3 text-sm leading-relaxed text-muted">{progressSummary(loading)}</p>
        {loading.phase === "stream" && (
          <p className="mt-0 mb-3 text-sm leading-relaxed text-muted">
            Uploaded {formatMB(loading.uploadedBytes)} model data. Generation and upload overlap;
            the preview appears only when both finish.
          </p>
        )}
        <ProgressBar
          value={indeterminate ? undefined : loading.completed}
          max={indeterminate ? undefined : loading.total}
          aria-label={progressLabel(loading)}
          className="mb-4"
        />
        <Button appearance="secondary" onClick={onCancel} className="mt-1.5!">
          Cancel
          <span className="ml-2.5 opacity-75 text-xs font-normal hidden sm:inline">Esc</span>
        </Button>
      </div>
    </div>
  )
}

export function PreviewStage({
  canvasRef,
  loaded,
  loading,
  onCancel,
  stageActive,
}: PreviewStageProps) {
  return (
    <section
      className={`absolute inset-0 bg-surface-secondary ${stageActive ? "visible pointer-events-auto" : "invisible pointer-events-none"}`}
      aria-label="Interactive 3D model"
      aria-hidden={!stageActive}
      aria-busy={Boolean(loading)}
    >
      <canvas
        ref={canvasRef}
        className="block w-full h-full [touch-action:none] outline-none focus-visible:outline-2 focus-visible:outline-accent focus-visible:-outline-offset-2"
        tabIndex={loaded ? 0 : -1}
        aria-label={loaded ? `3D preview of ${fileName(loaded.path)}` : "3D preview"}
        aria-describedby="canvas-controls"
      />
      <p id="canvas-controls" className="sr-only">
        Drag to orbit. Right or middle drag to pan. Scroll to zoom. Arrow keys orbit; Shift and
        arrow keys pan. Plus and minus zoom. F or Home fits the model.
      </p>
      {loaded && (
        <div
          className="absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-2 sm:gap-2.5 px-3 py-1.5 border border-border rounded-md bg-surface text-muted text-xs whitespace-nowrap pointer-events-none max-w-[calc(100%-2rem)] sm:max-w-none"
          aria-hidden="true"
        >
          Drag to orbit<span>·</span>Right-drag to pan<span>·</span>Scroll to zoom
        </div>
      )}
      {loading && <LoadingOverlay loading={loading} onCancel={onCancel} />}
    </section>
  )
}
