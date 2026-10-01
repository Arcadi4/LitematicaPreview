import { useCallback, useEffect, useRef, useState, type RefObject } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { getCurrentWindow } from "@tauri-apps/api/window"
import {
  appName,
  errorMessage,
  fileName,
  type Bootstrap,
  type DialogKind,
  type Loaded,
  type Loading,
  type MeshProgress,
  type NativeCommand,
  type Notice,
} from "./app-model"
import { savedPreviewSettings, type PreviewSettings } from "./preview-settings"
import { SchematicRenderer, type PreviewMetadata, type PreviewStreamEvent } from "./renderer"
import type { PreviewReadRange } from "./upload-layout"

export interface PreviewSession {
  actionBusy: boolean
  bootstrap: Bootstrap | null
  busy: boolean
  canvasRef: RefObject<HTMLCanvasElement | null>
  choosing: boolean
  dialog: DialogKind
  dialogOpen: boolean
  dragging: boolean
  errorDetails: string
  fit: () => void
  grid: boolean
  home: () => void
  loadDemo: (path: string) => void
  loaded: Loaded | null
  loading: Loading | null
  nativeAction: (command: NativeCommand) => void
  notice: Notice | null
  openDialog: (dialog: DialogKind) => void
  openFile: () => void
  previewMemory: number | null
  previewSettings: PreviewSettings
  setDialogOpen: (open: boolean) => void
  setNotice: (notice: Notice | null) => void
  toggleGrid: () => void
  updatePreviewSettings: (patch: Partial<PreviewSettings>) => void
  zoomIn: () => void
  zoomOut: () => void
}

export interface PreviewSessionOptions {
  /** The shell already recovered from a failure, so no path opens on its own. */
  initialError: string | undefined
}

/**
 * Owns the whole preview session: the native startup subscription, the WebGL
 * stage, the load pipeline and the ground grid. `generation` increases whenever a
 * request supersedes or cancels the previous one, so late native replies and
 * stale samples are ignored instead of overwriting the current view.
 */
export function usePreviewSession({ initialError }: PreviewSessionOptions) {
  const [dialog, setDialog] = useState<DialogKind>(initialError ? "error" : "settings")
  const [dialogOpen, setDialogOpen] = useState(Boolean(initialError))
  const [errorDetails, setErrorDetails] = useState(initialError || "")
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null)
  const [loading, setLoading] = useState<Loading | null>(null)
  const [previewMemory, setPreviewMemory] = useState<number | null>(null)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [dragging, setDragging] = useState(false)
  const [grid, setGrid] = useState(true)
  const [previewSettings, setPreviewSettings] = useState<PreviewSettings>(savedPreviewSettings)
  const [actionBusy, setActionBusy] = useState(false)
  const [choosing, setChoosing] = useState(false)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const rendererRef = useRef<SchematicRenderer | null>(null)
  const bootstrapRef = useRef<Bootstrap | null>(null)
  const gridRef = useRef(true)
  const previewSettingsRef = useRef(previewSettings)
  const mounted = useRef(false)
  const generation = useRef(0)
  const initialPathConsumed = useRef(Boolean(initialError))
  const choosingRef = useRef(false)
  const actionBusyRef = useRef(false)
  const titleQueue = useRef<Promise<void> | null>(null)
  const previewReadQueue = useRef<Promise<void> | null>(null)
  const memoryRequestRef = useRef<number | null>(null)
  const sampleMemoryRef = useRef<(() => void) | null>(null)

  const updatePreviewSettings = useCallback((patch: Partial<PreviewSettings>) => {
    const settings = { ...previewSettingsRef.current, ...patch }
    previewSettingsRef.current = settings
    setPreviewSettings(settings)
  }, [])

  const isCurrent = useCallback((id: number) => mounted.current && generation.current === id, [])

  // Serialize title changes so a delayed native call cannot restore an old filename.
  const updateTitle = useCallback(
    (path: string | null, id: number) => {
      const previous = titleQueue.current ?? Promise.resolve()
      titleQueue.current = previous.then(async () => {
        if (!isCurrent(id)) return
        try {
          await getCurrentWindow().setTitle(path ? `${fileName(path)} — ${appName}` : appName)
        } catch (error) {
          if (isCurrent(id))
            setNotice({
              intent: "error",
              message: `Could not update the window title: ${errorMessage(error)}`,
            })
        }
      })
    },
    [isCurrent],
  )

  const cancelNative = useCallback(
    (id: number) => {
      void invoke("cancel_load", { requestId: id }).catch((error: unknown) => {
        if (isCurrent(id))
          setNotice({
            intent: "error",
            message: `Could not cancel the load: ${errorMessage(error)}`,
          })
      })
    },
    [isCurrent],
  )

  const clearView = useCallback(() => {
    rendererRef.current?.clear()
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
  }, [])

  const home = useCallback(() => {
    const id = ++generation.current
    cancelNative(id)
    clearView()
    setNotice(null)
    updateTitle(null, id)
  }, [cancelNative, clearView, updateTitle])

  const graphicsFailed = useCallback(
    (message: string) => {
      if (!mounted.current) return
      const id = ++generation.current
      cancelNative(id)
      const renderer = rendererRef.current
      rendererRef.current = null
      renderer?.dispose()
      setLoaded(null)
      setLoading(null)
      setPreviewMemory(null)
      setNotice({
        intent: "error",
        message: `The preview could not be rendered. ${message}`,
      })
      updateTitle(null, id)
    },
    [cancelNative, updateTitle],
  )

  const fit = useCallback(() => rendererRef.current?.fit(), [])
  const zoomIn = useCallback(() => rendererRef.current?.zoom(0.88), [])
  const zoomOut = useCallback(() => rendererRef.current?.zoom(1.12), [])

  const toggleGrid = useCallback(() => {
    const visible = !gridRef.current
    gridRef.current = visible
    setGrid(visible)
    rendererRef.current?.setGrid(visible)
  }, [])

  // A fatal notice tears down the stage and shows the recovery dialog. Failures
  // while recovering are appended to that same dialog, never raised as a second.
  useEffect(() => {
    if (notice?.intent !== "error") return
    setErrorDetails(notice.message)
    setDialog("error")
    setDialogOpen(true)
    const id = ++generation.current
    const renderer = rendererRef.current
    rendererRef.current = null
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
    setNotice(null)
    const recoveryFailed = (error: unknown) => {
      if (isCurrent(id)) setErrorDetails((text) => `${text}\n\nRecovery: ${errorMessage(error)}`)
    }
    try {
      renderer?.dispose()
    } catch (error) {
      recoveryFailed(error)
    }
    void invoke("cancel_load", { requestId: id }).catch(recoveryFailed)
    void getCurrentWindow().setTitle(appName).catch(recoveryFailed)
  }, [notice, isCurrent])

  // Surface uncaught failures in the same place as every other notice instead of
  // losing them to the default browser reporting.
  useEffect(() => {
    const onError = (event: ErrorEvent) => {
      event.preventDefault()
      setNotice({ intent: "error", message: errorMessage(event.error || event.message) })
    }
    const onRejection = (event: PromiseRejectionEvent) => {
      event.preventDefault()
      setNotice({ intent: "error", message: errorMessage(event.reason) })
    }
    window.addEventListener("error", onError)
    window.addEventListener("unhandledrejection", onRejection)
    return () => {
      window.removeEventListener("error", onError)
      window.removeEventListener("unhandledrejection", onRejection)
    }
  }, [])

  const loadPath = useCallback(
    async (path: string) => {
      if (!mounted.current) return
      // Read current settings without rebuilding startup and drag-and-drop subscriptions.
      // This request keeps its own snapshot even if settings change during decoding.
      const settings = previewSettingsRef.current
      const speedFirst = settings.multithreadingEnabled && !settings.conservativeMemoryScheduling
      const options = {
        memoryLimitMB: settings.memoryLimitEnabled ? settings.memoryLimitMB : null,
        chunkSize: settings.chunkingEnabled ? settings.chunkSize : null,
        threadCount: settings.multithreadingEnabled ? settings.threadCount : null,
        speedFirst,
      }
      const id = ++generation.current
      clearView()
      setNotice(null)
      updateTitle(null, id)
      const supported = bootstrapRef.current?.extensions.some((extension) =>
        path.toLowerCase().endsWith(extension.toLowerCase()),
      )
      if (!supported) {
        cancelNative(id)
        setNotice({
          intent: "error",
          message: `“${fileName(path)}” is not a supported schematic. Choose one of the formats listed below.`,
        })
        return
      }
      setLoading({
        path,
        requestId: id,
        phase: "decode",
        completed: 0,
        total: 0,
        uploadedBytes: 0,
      })
      const started = performance.now()
      let unlistenProgress: (() => void) | undefined
      try {
        unlistenProgress = await listen<MeshProgress>("preview-progress", ({ payload }) => {
          if (!isCurrent(id) || payload.requestId !== id || payload.phase !== "mesh") return
          if (!Number.isSafeInteger(payload.total) || payload.total <= 0) return
          if (
            !Number.isSafeInteger(payload.completed) ||
            payload.completed < 0 ||
            payload.completed > payload.total
          )
            return
          setLoading((previous) =>
            previous?.requestId === id && previous.phase !== "upload"
              ? {
                  ...previous,
                  phase: options.threadCount === null ? "mesh" : "stream",
                  completed: payload.completed,
                  total: payload.total,
                }
              : previous,
          )
        })
        if (!isCurrent(id)) return
        // Serial mode still completes native generation before allocating GPU state.
        const descriptor =
          options.threadCount === null
            ? await invoke<PreviewMetadata>("load_preview", { path, requestId: id, options })
            : null
        if (!isCurrent(id)) return
        let renderer = rendererRef.current
        if (!renderer) {
          if (!canvasRef.current) throw new Error("The preview canvas is unavailable.")
          renderer = new SchematicRenderer(canvasRef.current, graphicsFailed)
          if (!isCurrent(id)) {
            renderer.dispose()
            return
          }
          rendererRef.current = renderer
          renderer.setGrid(gridRef.current)
        }
        const readRanges = (batchId: number | null, ranges: readonly PreviewReadRange[]) => {
          if (!isCurrent(id)) return Promise.reject(new Error("Cancelled"))
          return invoke<ArrayBuffer>("read_preview", {
            requestId: id,
            batchId,
            ranges: ranges.map(({ bufferId, offset, length }) => ({ bufferId, offset, length })),
          })
        }
        let lastUploadUpdate = 0
        let metadata: PreviewMetadata
        if (options.threadCount !== null) {
          // The producer starts before the first pull and runs ahead only within its bounded queue.
          await invoke("start_preview", { path, requestId: id, options })
          if (!isCurrent(id)) return
          metadata = await renderer.loadStream(
            async (previousBatchId) => {
              if (!isCurrent(id)) throw new Error("Cancelled")
              const event = await invoke<PreviewStreamEvent>("next_preview", {
                requestId: id,
                previousBatchId,
              })
              if (!isCurrent(id)) throw new Error("Cancelled")
              return event
            },
            readRanges,
            () => isCurrent(id),
            (uploadedBytes) => {
              if (!isCurrent(id)) return
              const now = performance.now()
              if (now - lastUploadUpdate < 150) return
              lastUploadUpdate = now
              setLoading((previous) =>
                previous?.requestId === id
                  ? { ...previous, phase: "stream", uploadedBytes }
                  : previous,
              )
            },
            options.threadCount,
            options.speedFirst,
          )
        } else {
          if (!descriptor) throw new Error("The preview metadata is unavailable.")
          setLoading({
            path,
            requestId: id,
            phase: "upload",
            completed: 0,
            total: descriptor.byteLength,
            uploadedBytes: 0,
          })
          metadata = await renderer.load(
            descriptor,
            (ranges) => {
              const previous = previewReadQueue.current ?? Promise.resolve()
              const read = previous.then(() => readRanges(null, ranges))
              previewReadQueue.current = read.then(
                () => {},
                () => {},
              )
              return read
            },
            () => isCurrent(id),
            (completed, total) => {
              if (!isCurrent(id)) return
              const now = performance.now()
              if (completed !== total && now - lastUploadUpdate < 150) return
              lastUploadUpdate = now
              setLoading((previous) =>
                previous?.requestId === id && previous.phase === "upload"
                  ? { ...previous, completed, total, uploadedBytes: completed }
                  : previous,
              )
            },
            null,
            false,
          )
        }
        if (!isCurrent(id)) return
        setLoaded({
          path,
          metadata,
          seconds: (performance.now() - started) / 1000,
        })
        setLoading(null)
        setPreviewMemory(null)
        updateTitle(path, id)
        canvasRef.current?.focus({ preventScroll: true })
      } catch (error) {
        if (!isCurrent(id)) return
        rendererRef.current?.clear()
        setLoading(null)
        setPreviewMemory(null)
        if (errorMessage(error) !== "Cancelled")
          setNotice({ intent: "error", message: `${path}\n\n${errorMessage(error)}` })
      } finally {
        unlistenProgress?.()
        try {
          await invoke("release_preview", { requestId: id })
        } catch (error) {
          if (isCurrent(id))
            setNotice({
              intent: "error",
              message: `Could not finish loading the preview: ${errorMessage(error)}`,
            })
        }
      }
    },
    [cancelNative, clearView, graphicsFailed, isCurrent, updateTitle],
  )

  const chooseFile = useCallback(async () => {
    if (!mounted.current || !bootstrapRef.current || choosingRef.current) return
    choosingRef.current = true
    setChoosing(true)
    const id = generation.current
    // Dismissing the picker must leave the current preview or load intact.
    try {
      const path = await invoke<string | null>("choose_file")
      if (path && isCurrent(id)) void loadPath(path)
    } catch (error) {
      if (isCurrent(id)) setNotice({ intent: "error", message: errorMessage(error) })
    } finally {
      choosingRef.current = false
      if (mounted.current) setChoosing(false)
    }
  }, [isCurrent, loadPath])

  const loadDemo = useCallback(
    (path: string) => {
      void loadPath(path)
    },
    [loadPath],
  )

  const openFile = useCallback(() => {
    void chooseFile()
  }, [chooseFile])

  const nativeAction = useCallback(
    async (command: NativeCommand) => {
      if (actionBusyRef.current) return
      actionBusyRef.current = true
      setActionBusy(true)
      const id = generation.current
      try {
        await invoke(command)
        if (isCurrent(id)) {
          setNotice({
            intent: "success",
            message:
              command === "register_associations"
                ? "File associations registered. Choose Litematica Preview for your schematic files in Windows Settings."
                : command === "unregister_associations"
                  ? "File associations for this copy of Litematica Preview were removed."
                  : "The bundled licenses folder has been opened.",
          })
        }
      } catch (error) {
        if (isCurrent(id)) setNotice({ intent: "error", message: errorMessage(error) })
      } finally {
        actionBusyRef.current = false
        setActionBusy(false)
      }
    },
    [isCurrent],
  )

  useEffect(() => {
    mounted.current = true
    // Keep restoration opted in even if a lost context makes the renderer
    // dispose itself before the browser dispatches webglcontextlost.
    const canvas = canvasRef.current
    const allowContextRestore = (event: Event) => event.preventDefault()
    canvas?.addEventListener("webglcontextlost", allowContextRestore)
    let active = true
    let unlisten: (() => void) | undefined
    const initialGeneration = generation.current
    void invoke<Bootstrap>("bootstrap")
      .then((result) => {
        if (!active) return
        const openInitialPath = isCurrent(initialGeneration)
        generation.current = Math.max(generation.current, result.requestId)
        bootstrapRef.current = result
        setBootstrap(result)
        const settings = previewSettingsRef.current
        updatePreviewSettings({
          multithreadingEnabled:
            settings.multithreadingEnabled &&
            settings.chunkingEnabled &&
            result.maxWorkerThreads >= 2,
          threadCount: Math.max(2, Math.min(settings.threadCount, result.maxWorkerThreads)),
        })
        if (!initialPathConsumed.current) {
          initialPathConsumed.current = true
          if (result.initialPath && openInitialPath) void loadPath(result.initialPath)
        }
      })
      .catch((error: unknown) => {
        if (active)
          setNotice({
            intent: "error",
            message: `Could not initialize the application: ${errorMessage(error)}`,
          })
      })
    void getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (!active) return
        if (payload.type === "leave") {
          setDragging(false)
        } else if (payload.type === "enter" || payload.type === "over") {
          setDragging(true)
        } else if (payload.type === "drop") {
          setDragging(false)
          const extensions = bootstrapRef.current?.extensions
          if (!extensions) {
            setNotice({
              intent: "info",
              message:
                "The application is still starting. Please drop your file again in a moment.",
            })
            return
          }
          const path = payload.paths.find((candidate) =>
            extensions.some((extension) =>
              candidate.toLowerCase().endsWith(extension.toLowerCase()),
            ),
          )
          if (path) void loadPath(path)
          else
            setNotice({
              intent: "error",
              message: `No supported schematic was dropped. Supported formats: ${extensions.join(", ")}.`,
            })
        }
      })
      .then((dispose) => {
        if (active) unlisten = dispose
        else dispose()
      })
      .catch((error: unknown) => {
        if (active)
          setNotice({
            intent: "error",
            message: `Drag and drop is unavailable. You can still use Open. ${errorMessage(error)}`,
          })
      })
    return () => {
      active = false
      mounted.current = false
      const id = ++generation.current
      cancelNative(id)
      unlisten?.()
      rendererRef.current?.dispose()
      rendererRef.current = null
      canvas?.removeEventListener("webglcontextlost", allowContextRestore)
    }
  }, [cancelNative, isCurrent, loadPath, updatePreviewSettings])

  // The sampler subscribes once for the lifetime of the app: the active request
  // id is mirrored into a ref, so starting or cancelling a load never
  // re-subscribes the interval and a late sample can never write stale memory.
  useEffect(() => {
    let inFlight = false
    const sample = async () => {
      const id = memoryRequestRef.current
      if (id === null || inFlight) return
      inFlight = true
      try {
        const memory = await invoke<number | null>("preview_memory", { requestId: id })
        if (memoryRequestRef.current === id) setPreviewMemory(memory)
      } catch {
        if (memoryRequestRef.current === id) setPreviewMemory(null)
      } finally {
        inFlight = false
      }
    }
    sampleMemoryRef.current = () => void sample()
    const timer = window.setInterval(sampleMemoryRef.current, 500)
    return () => {
      sampleMemoryRef.current = null
      window.clearInterval(timer)
    }
  }, [])

  useEffect(() => {
    memoryRequestRef.current = loading?.requestId ?? null
    sampleMemoryRef.current?.()
  }, [loading])

  const runNativeAction = useCallback(
    (command: NativeCommand) => {
      void nativeAction(command)
    },
    [nativeAction],
  )

  const openDialog = useCallback((next: DialogKind) => {
    setDialog(next)
    setDialogOpen(true)
  }, [])

  return {
    actionBusy,
    bootstrap,
    busy: !bootstrap || choosing,
    canvasRef,
    choosing,
    dialog,
    dialogOpen,
    dragging,
    errorDetails,
    fit,
    grid,
    home,
    loadDemo,
    loaded,
    loading,
    nativeAction: runNativeAction,
    notice,
    openDialog,
    openFile,
    previewMemory,
    previewSettings,
    setDialogOpen,
    setNotice,
    toggleGrid,
    updatePreviewSettings,
    zoomIn,
    zoomOut,
  } satisfies PreviewSession
}
