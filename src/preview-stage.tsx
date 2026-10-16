import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react"
import { Button, ProgressBar } from "@fluentui/react-components"
import { Cube24Regular } from "@fluentui/react-icons"
import { fileName, numbers, type Loaded, type Loading, type ViewportCommand } from "./app-model"

export interface PreviewStageProps {
  loaded: Loaded | null
  loading: Loading | null
  onCancel: () => void
  onViewportInput: (command: ViewportCommand) => void
  stageActive: boolean
  viewportRef: RefObject<HTMLDivElement | null>
}

function percent(completed: number, total: number) {
  return Math.round((completed / total) * 100)
}

function progressSummary(loading: Loading) {
  if (loading.phase === "decode") return "Reading blocks locally."
  if (loading.total <= 0) return "Generating geometry."
  return `Generating geometry: ${numbers.format(loading.completed)} / ${numbers.format(loading.total)} chunks (${percent(loading.completed, loading.total)}%).`
}

function progressLabel(loading: Loading) {
  return loading.phase === "decode" ? "Reading blocks" : "Generating geometry"
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
  loaded,
  loading,
  onCancel,
  onViewportInput,
  stageActive,
  viewportRef,
}: PreviewStageProps) {
  // Tracks the pointer owning orbit/pan capture.
  const drag = useRef<{ pointerId: number; button: number; x: number; y: number } | null>(null)

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current !== null || event.button < 0 || event.button > 2) return
    event.currentTarget.focus({ preventScroll: true })
    drag.current = {
      pointerId: event.pointerId,
      button: event.button,
      x: event.clientX,
      y: event.clientY,
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const active = drag.current
    if (active === null || event.pointerId !== active.pointerId) return
    const dx = event.clientX - active.x
    const dy = event.clientY - active.y
    active.x = event.clientX
    active.y = event.clientY
    if (!loaded || (dx === 0 && dy === 0)) return
    if (active.button === 0) onViewportInput({ kind: "orbit", dx, dy })
    else onViewportInput({ kind: "pan", dx, dy })
  }

  const endPointer = (event: PointerEvent<HTMLDivElement>) => {
    const active = drag.current
    if (active === null || event.pointerId !== active.pointerId) return
    drag.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId)
  }

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!loaded || event.ctrlKey || event.altKey || event.metaKey) return
    let dx = 0
    let dy = 0
    switch (event.key) {
      case "ArrowLeft":
        dx = -18
        break
      case "ArrowRight":
        dx = 18
        break
      case "ArrowUp":
        dy = -18
        break
      case "ArrowDown":
        dy = 18
        break
      case "+":
      case "=":
        event.preventDefault()
        onViewportInput({ kind: "zoom", factor: 0.88 })
        return
      case "-":
      case "_":
        event.preventDefault()
        onViewportInput({ kind: "zoom", factor: 1.12 })
        return
      case "f":
      case "F":
      case "Home":
        event.preventDefault()
        onViewportInput({ kind: "fit" })
        return
      default:
        return
    }
    event.preventDefault()
    onViewportInput({ kind: "arrow", dx, dy, panning: event.shiftKey })
  }

  // A native non-passive listener lets zoom prevent the default wheel action.
  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const onWheel = (event: WheelEvent) => {
      if (!loaded) return
      event.preventDefault()
      onViewportInput({ kind: "wheel", deltaY: event.deltaY, deltaMode: event.deltaMode })
    }
    element.addEventListener("wheel", onWheel, { passive: false })
    return () => element.removeEventListener("wheel", onWheel)
  }, [loaded, onViewportInput, viewportRef])

  // ResizeObserver catches chrome layout changes without a Tauri window event.
  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const report = () => {
      const rect = element.getBoundingClientRect()
      onViewportInput({
        kind: "stage",
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height,
      })
    }
    report()
    const observer = new ResizeObserver(report)
    observer.observe(element)
    return () => observer.disconnect()
  }, [onViewportInput, viewportRef])

  return (
    <section
      className={`absolute inset-0 ${stageActive ? "visible pointer-events-auto" : "invisible pointer-events-none"}`}
      aria-label="Interactive 3D model"
      aria-hidden={!stageActive}
      aria-busy={Boolean(loading)}
    >
      <div
        ref={viewportRef}
        role="application"
        className="block w-full h-full bg-transparent [touch-action:none] outline-none focus-visible:outline-2 focus-visible:outline-accent focus-visible:-outline-offset-2"
        tabIndex={loaded ? 0 : -1}
        aria-label={loaded ? `3D preview of ${fileName(loaded.path)}` : "3D preview"}
        aria-describedby="canvas-controls"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endPointer}
        onPointerCancel={endPointer}
        onLostPointerCapture={endPointer}
        onKeyDown={onKeyDown}
        onContextMenu={(event) => event.preventDefault()}
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
