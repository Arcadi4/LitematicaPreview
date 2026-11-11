import {
  useEffect,
  useEffectEvent,
  useRef,
  type KeyboardEvent,
  type PointerEvent,
  type RefObject,
} from "react"
import { Button, ProgressBar } from "@fluentui/react-components"
import { fileName, numbers, type Loaded, type Loading, type ViewportCommand } from "./app-model"
import { FlyMotion, capturePointer, requestPointerLock } from "./fly-controls"

export interface PreviewStageProps {
  flyMode: boolean
  loaded: Loaded | null
  loading: Loading | null
  onCancel: () => void
  onExitFly: () => void
  onViewportInput: (command: ViewportCommand) => void
  stageActive: boolean
  viewportRef: RefObject<HTMLDivElement | null>
}

function percent(completed: number, total: number) {
  return Math.round((completed / total) * 100)
}

function progressSummary(loading: Loading) {
  if (loading.phase === "decode") return "Reading blocks."
  if (loading.phase === "upload") return "Finishing GPU uploads."
  if (loading.total <= 0) return "Generating geometry."
  return `Source chunks processed: ${numbers.format(loading.completed)} / ${numbers.format(loading.total)} (${percent(loading.completed, loading.total)}%).`
}

function progressLabel(loading: Loading) {
  if (loading.phase === "decode") return "Reading blocks"
  return loading.phase === "upload" ? "Finishing GPU uploads" : "Generating geometry"
}

function LoadingProgress({ loading, onCancel }: { loading: Loading; onCancel: () => void }) {
  const indeterminate = loading.phase === "upload" || loading.total === 0
  return (
    <div className="absolute top-4 inset-x-4 flex justify-center pointer-events-none">
      <div
        className="max-w-lg w-full px-4 py-3 border border-border rounded-lg bg-surface/85 shadow-md"
        role="status"
        aria-live="polite"
      >
        <div className="flex items-center justify-between gap-3 mb-2">
          <div className="min-w-0">
            <p className="m-0 text-sm font-semibold">{progressLabel(loading)}</p>
            <p className="m-0 text-xs text-muted truncate" title={loading.path}>
              {fileName(loading.path)}
            </p>
          </div>
          <Button
            appearance="secondary"
            onClick={onCancel}
            className="pointer-events-auto shrink-0"
          >
            Cancel
            <span className="ml-2 opacity-75 text-xs font-normal hidden sm:inline">Esc</span>
          </Button>
        </div>
        <ProgressBar
          value={indeterminate ? undefined : loading.completed}
          max={indeterminate ? undefined : loading.total}
          aria-label={progressLabel(loading)}
        />
        <p className="mt-2 mb-0 text-xs leading-relaxed text-muted">{progressSummary(loading)}</p>
      </div>
    </div>
  )
}

export function PreviewStage({
  flyMode,
  loaded,
  loading,
  onCancel,
  onExitFly,
  onViewportInput,
  stageActive,
  viewportRef,
}: PreviewStageProps) {
  const previewPath = loaded?.path ?? loading?.path
  // The pointer that owns the current drag, mirroring the native renderer's
  // single-pointer orbit/pan capture.
  const drag = useRef<{ pointerId: number; button: number; x: number; y: number } | null>(null)
  const motion = useRef<FlyMotion | null>(null)

  const flyMotion = () => {
    if (motion.current === null)
      motion.current = new FlyMotion((direction, seconds) =>
        onViewportInput({ kind: "flyMove", ...direction, seconds }),
      )
    return motion.current
  }

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current !== null || event.button < 0 || event.button > 2) return
    event.currentTarget.focus({ preventScroll: true })
    drag.current = {
      pointerId: event.pointerId,
      button: event.button,
      x: event.clientX,
      y: event.clientY,
    }
    capturePointer(event.currentTarget, event.pointerId)
    if (flyMode && document.pointerLockElement !== event.currentTarget)
      requestPointerLock(event.currentTarget)
    event.preventDefault()
  }

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const active = drag.current
    if (active === null || event.pointerId !== active.pointerId) return
    const dx = event.clientX - active.x
    const dy = event.clientY - active.y
    active.x = event.clientX
    active.y = event.clientY
    if (!stageActive || (dx === 0 && dy === 0)) return
    if (flyMode) {
      if (document.pointerLockElement !== event.currentTarget)
        onViewportInput({ kind: "flyLook", dx, dy })
    } else if (active.button === 0) onViewportInput({ kind: "orbit", dx, dy })
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
    if (!stageActive) return
    if (flyMode) {
      if (event.key === "Escape") {
        event.preventDefault()
        onExitFly()
      } else if (event.key === "f" || event.key === "F" || event.key === "Home") {
        event.preventDefault()
        onExitFly()
        onViewportInput({ kind: "fit" })
      } else if (!event.altKey && !event.metaKey && flyMotion().press(event.code)) {
        event.preventDefault()
      }
      return
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return
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

  const onKeyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (flyMode) motion.current?.release(event.code)
  }

  const onBlur = () => {
    motion.current?.stop()
    drag.current = null
  }

  const handleFlyMouseMove = useEffectEvent((event: MouseEvent, element: HTMLElement) => {
    if (!flyMode || document.pointerLockElement !== element) return
    if (event.movementX === 0 && event.movementY === 0) return
    onViewportInput({ kind: "flyLook", dx: event.movementX, dy: event.movementY })
  })
  const handleFlyLockChange = useEffectEvent((element: HTMLElement) => {
    if (flyMode && document.pointerLockElement !== element) onExitFly()
  })

  // Fly mode owns the mouse only while it holds the pointer lock; losing the
  // lock (Esc, window switch) returns to orbit.
  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const onMouseMove = (event: MouseEvent) => handleFlyMouseMove(event, element)
    const onLockChange = () => handleFlyLockChange(element)
    document.addEventListener("mousemove", onMouseMove)
    document.addEventListener("pointerlockchange", onLockChange)
    return () => {
      document.removeEventListener("mousemove", onMouseMove)
      document.removeEventListener("pointerlockchange", onLockChange)
    }
  }, [viewportRef])

  useEffect(() => {
    if (flyMode) return
    motion.current?.stop()
    drag.current = null
  }, [flyMode])

  useEffect(() => () => motion.current?.stop(), [])

  // Leaving the window or hiding the page must not keep moving a camera whose
  // held keys can no longer receive their keyup.
  useEffect(() => {
    if (!flyMode) return
    const exit = () => {
      motion.current?.stop()
      onExitFly()
    }
    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") exit()
    }
    window.addEventListener("blur", exit)
    document.addEventListener("visibilitychange", onVisibilityChange)
    return () => {
      window.removeEventListener("blur", exit)
      document.removeEventListener("visibilitychange", onVisibilityChange)
    }
  }, [flyMode, onExitFly])

  // A native non-passive listener lets zoom prevent the default wheel action.
  useEffect(() => {
    const element = viewportRef.current
    if (!element) return
    const onWheel = (event: WheelEvent) => {
      if (!stageActive || flyMode) return
      event.preventDefault()
      onViewportInput({ kind: "wheel", deltaY: event.deltaY, deltaMode: event.deltaMode })
    }
    element.addEventListener("wheel", onWheel, { passive: false })
    return () => element.removeEventListener("wheel", onWheel)
  }, [flyMode, stageActive, onViewportInput, viewportRef])

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
        tabIndex={stageActive ? 0 : -1}
        aria-label={previewPath ? `3D preview of ${fileName(previewPath)}` : "3D preview"}
        aria-describedby="canvas-controls"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endPointer}
        onPointerCancel={endPointer}
        onLostPointerCapture={endPointer}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onBlur={onBlur}
        onContextMenu={(event) => event.preventDefault()}
      />
      <p id="canvas-controls" className="sr-only">
        {flyMode
          ? "Spectator fly mode. W and S move forward and back, A and D move sideways, Space rises, and Control descends. Move the mouse to look around; click the viewport to capture the pointer, or drag to look when capture is unavailable. Press V or Escape to return to orbit."
          : "Drag to orbit. Right or middle drag to pan. Scroll to zoom. Arrow keys orbit; Shift and arrow keys pan. Plus and minus zoom. F or Home fits the model."}
      </p>
      {stageActive && (
        <div
          className="absolute bottom-4 left-1/2 -translate-x-1/2 flex items-center gap-2 sm:gap-2.5 px-3 py-1.5 border border-border rounded-md bg-surface text-muted text-xs whitespace-nowrap pointer-events-none max-w-[calc(100%-2rem)] sm:max-w-none"
          aria-hidden="true"
        >
          {flyMode ? (
            <>
              WASD move<span>·</span>Space / Ctrl up-down<span>·</span>Mouse looks
              <span>·</span>V orbits
            </>
          ) : (
            <>
              Drag to orbit<span>·</span>Right-drag to pan<span>·</span>Scroll to zoom
            </>
          )}
        </div>
      )}
      {loading && <LoadingProgress loading={loading} onCancel={onCancel} />}
    </section>
  )
}
