import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react"
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
  type NativeCommand,
  type Notice,
  type PreviewFailed,
  type PreviewLoaded,
  type PreviewProgress,
  type ViewportCommand,
  type ViewportFailed,
} from "./app-model"
import { savedPreviewSettings, type PreviewSettings } from "./preview-settings"
import { ViewportInputQueue } from "./viewport-input"

export interface PreviewSession {
  actionBusy: boolean
  bootstrap: Bootstrap | null
  busy: boolean
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
  viewportInput: (command: ViewportCommand) => void
  viewportRef: RefObject<HTMLDivElement | null>
  zoomIn: () => void
  zoomOut: () => void
}

export interface PreviewSessionOptions {
  /** The shell already recovered from a failure, so no path opens on its own. */
  initialError: string | undefined
}

/**
 * Owns the whole preview session: the native startup subscription, the load
 * pipeline and the ground grid. The schematic is rendered by the native
 * viewport behind the WebView, so this hook only orchestrates: `generation`
 * increases whenever a request supersedes or cancels the previous one, so late
 * native replies and stale samples are ignored instead of overwriting the
 * current view.
 */
export function usePreviewSession({ initialError }: PreviewSessionOptions) {
  const [dialog, setDialog] = useState<DialogKind>(initialError ? "error" : "settings")
  const [dialogOpen, setDialogOpen] = useState(Boolean(initialError))
  const [errorDetails, setErrorDetails] = useState(initialError || "")
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null)
  const [loading, setLoading] = useState<Loading | null>(null)
  const [previewMemory, setPreviewMemory] = useState<number | null>(null)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [rawNotice, setRawNotice] = useState<Notice | null>(null)
  const [viewportError, setViewportError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const [grid, setGrid] = useState(true)
  const [previewSettings, setPreviewSettings] = useState<PreviewSettings>(savedPreviewSettings)
  const [actionBusy, setActionBusy] = useState(false)
  const [choosing, setChoosing] = useState(false)
  const viewportRef = useRef<HTMLDivElement>(null)
  const bootstrapRef = useRef<Bootstrap | null>(null)
  const gridRef = useRef(true)
  const previewSettingsRef = useRef(previewSettings)
  const mounted = useRef(false)
  const generation = useRef(0)
  const initialPathConsumed = useRef(Boolean(initialError))
  const choosingRef = useRef(false)
  const actionBusyRef = useRef(false)
  const titleQueue = useRef<Promise<void> | null>(null)
  const memoryRequestRef = useRef<number | null>(null)
  const sampleMemoryRef = useRef<(() => void) | null>(null)
  // The only request the load events may resolve. A superseded or cancelled
  // load leaves it null (or stale), so its late events are dropped.
  const pendingRequest = useRef<{ id: number; path: string; started: number } | null>(null)

  // The native viewport failed to initialize: every preview keeps failing, so
  // the message survives the notice resets later loads perform.
  const notice = useMemo(
    () =>
      viewportError !== null ? ({ intent: "error", message: viewportError } as const) : rawNotice,
    [viewportError, rawNotice],
  )

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
            setRawNotice({
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
          setRawNotice({
            intent: "error",
            message: `Could not cancel the load: ${errorMessage(error)}`,
          })
      })
    },
    [isCurrent],
  )

  const viewportQueue = useMemo(
    () =>
      new ViewportInputQueue(
        (commands) => invoke("viewport_input", { commands }),
        (error) => setRawNotice({ intent: "error", message: errorMessage(error) }),
      ),
    [],
  )
  useEffect(() => () => viewportQueue.clear(), [viewportQueue])
  const viewportInput = useCallback(
    (command: ViewportCommand) => viewportQueue.push(command),
    [viewportQueue],
  )

  const clearView = useCallback(() => {
    viewportQueue.clear()
    pendingRequest.current = null
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
  }, [viewportQueue])

  const home = useCallback(() => {
    const id = ++generation.current
    cancelNative(id)
    clearView()
    setRawNotice(null)
    updateTitle(null, id)
  }, [cancelNative, clearView, updateTitle])

  const fit = useCallback(() => viewportInput({ kind: "fit" }), [viewportInput])
  const zoomIn = useCallback(() => viewportInput({ kind: "zoom", factor: 0.88 }), [viewportInput])
  const zoomOut = useCallback(() => viewportInput({ kind: "zoom", factor: 1.12 }), [viewportInput])

  const toggleGrid = useCallback(() => {
    const visible = !gridRef.current
    gridRef.current = visible
    setGrid(visible)
    viewportInput({ kind: "grid", visible })
  }, [viewportInput])

  // A fatal notice tears down the stage and shows the recovery dialog. Failures
  // while recovering are appended to that same dialog, never raised as a second.
  // The durable viewport failure only shows in the notice bar, so this effect
  // keys on the raw notice instead of the composed one.
  useEffect(() => {
    if (rawNotice?.intent !== "error") return
    setErrorDetails(rawNotice.message)
    setDialog("error")
    setDialogOpen(true)
    const id = ++generation.current
    pendingRequest.current = null
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
    setRawNotice(null)
    const recoveryFailed = (error: unknown) => {
      if (isCurrent(id)) setErrorDetails((text) => `${text}\n\nRecovery: ${errorMessage(error)}`)
    }
    void invoke("cancel_load", { requestId: id }).catch(recoveryFailed)
    void getCurrentWindow().setTitle(appName).catch(recoveryFailed)
  }, [rawNotice, isCurrent])

  // Surface uncaught failures in the same place as every other notice instead of
  // losing them to the default browser reporting.
  useEffect(() => {
    const onError = (event: ErrorEvent) => {
      event.preventDefault()
      setRawNotice({ intent: "error", message: errorMessage(event.error || event.message) })
    }
    const onRejection = (event: PromiseRejectionEvent) => {
      event.preventDefault()
      setRawNotice({ intent: "error", message: errorMessage(event.reason) })
    }
    window.addEventListener("error", onError)
    window.addEventListener("unhandledrejection", onRejection)
    return () => {
      window.removeEventListener("error", onError)
      window.removeEventListener("unhandledrejection", onRejection)
    }
  }, [])

  // Load completion is event-driven for both the serial and the streaming
  // command. The listeners live for the whole session so a native event can
  // never race the command promise's cleanup, and they resolve only the request
  // that is still current.
  useEffect(() => {
    let active = true
    const unlisteners: Array<() => void> = []
    const track = (dispose: () => void) => {
      if (active) unlisteners.push(dispose)
      else dispose()
    }
    void listen<PreviewLoaded>("preview-loaded", ({ payload }) => {
      const request = pendingRequest.current
      if (!request || request.id !== payload.requestId || !isCurrent(payload.requestId)) return
      pendingRequest.current = null
      setLoaded({
        path: request.path,
        metadata: payload.metadata,
        seconds: (performance.now() - request.started) / 1000,
      })
      setLoading(null)
      setPreviewMemory(null)
      updateTitle(request.path, request.id)
      viewportRef.current?.focus({ preventScroll: true })
    }).then(track)
    void listen<PreviewFailed>("preview-failed", ({ payload }) => {
      const request = pendingRequest.current
      if (!request || request.id !== payload.requestId || !isCurrent(payload.requestId)) return
      pendingRequest.current = null
      setLoading(null)
      setPreviewMemory(null)
      setRawNotice({ intent: "error", message: `${request.path}\n\n${payload.message}` })
    }).then(track)
    void listen<ViewportFailed>("viewport-failed", ({ payload }) => {
      setViewportError(payload.message)
    }).then(track)
    return () => {
      active = false
      for (const dispose of unlisteners) dispose()
    }
  }, [isCurrent, updateTitle])

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
      const started = performance.now()
      clearView()
      setRawNotice(null)
      updateTitle(null, id)
      const supported = bootstrapRef.current?.extensions.some((extension) =>
        path.toLowerCase().endsWith(extension.toLowerCase()),
      )
      if (!supported) {
        cancelNative(id)
        setRawNotice({
          intent: "error",
          message: `“${fileName(path)}” is not a supported schematic. Choose one of the formats listed below.`,
        })
        return
      }
      setLoading({ path, requestId: id, phase: "decode", completed: 0, total: 0 })
      pendingRequest.current = { id, path, started }
      let unlistenProgress: (() => void) | undefined
      try {
        unlistenProgress = await listen<PreviewProgress>("preview-progress", ({ payload }) => {
          if (!isCurrent(id) || payload.requestId !== id || payload.phase !== "mesh") return
          if (!Number.isSafeInteger(payload.total) || payload.total <= 0) return
          if (
            !Number.isSafeInteger(payload.completed) ||
            payload.completed < 0 ||
            payload.completed > payload.total
          )
            return
          setLoading((previous) =>
            previous?.requestId === id
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
        // Serial mode decodes and meshes inside the command; streaming mode
        // starts the native producer. Both hand control to the preview events.
        await invoke(options.threadCount === null ? "load_preview" : "start_preview", {
          path,
          requestId: id,
          options,
        })
      } catch (error) {
        if (!isCurrent(id)) return
        pendingRequest.current = null
        setLoading(null)
        setPreviewMemory(null)
        if (errorMessage(error) !== "Cancelled")
          setRawNotice({ intent: "error", message: `${path}\n\n${errorMessage(error)}` })
      } finally {
        unlistenProgress?.()
      }
    },
    [cancelNative, clearView, isCurrent, updateTitle],
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
      if (isCurrent(id)) setRawNotice({ intent: "error", message: errorMessage(error) })
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
          setRawNotice({
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
        if (isCurrent(id)) setRawNotice({ intent: "error", message: errorMessage(error) })
      } finally {
        actionBusyRef.current = false
        setActionBusy(false)
      }
    },
    [isCurrent],
  )

  useEffect(() => {
    mounted.current = true
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
          setRawNotice({
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
            setRawNotice({
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
            setRawNotice({
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
          setRawNotice({
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
      pendingRequest.current = null
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
    setNotice: setRawNotice,
    toggleGrid,
    updatePreviewSettings,
    viewportInput,
    viewportRef,
    zoomIn,
    zoomOut,
  } satisfies PreviewSession
}
