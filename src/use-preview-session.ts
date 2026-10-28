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
  type ImportedTexturePacks,
  type Loaded,
  type Loading,
  type NativeCommand,
  type Notice,
  type PreviewFailed,
  type PreviewImageAction,
  type PreviewLoaded,
  type PreviewProgress,
  type TexturePackState,
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
  copyImage: () => void
  dialog: DialogKind
  dialogOpen: boolean
  dragging: boolean
  errorDetails: string
  exportImage: () => void
  fit: () => void
  grid: boolean
  home: () => void
  imageAction: PreviewImageAction | null
  importTexturePacks: () => void
  loadDemo: (path: string) => void
  loaded: Loaded | null
  loading: Loading | null
  nativeAction: (command: NativeCommand) => void
  notice: Notice | null
  openDialog: (dialog: DialogKind) => void
  openFile: () => void
  previewMemory: number | null
  previewSettings: PreviewSettings
  removeTexturePack: (id: string) => void
  setDialogOpen: (open: boolean) => void
  setNotice: (notice: Notice | null) => void
  setTexturePackEnabled: (id: string, enabled: boolean) => void
  setTexturePackOrder: (ids: string[]) => void
  texturePackBusy: boolean
  texturePacks: TexturePackState
  toggleGrid: () => void
  updatePreviewSettings: (patch: Partial<PreviewSettings>) => void
  viewportInput: (command: ViewportCommand) => void
  viewportRef: RefObject<HTMLDivElement | null>
  zoomIn: () => void
  zoomOut: () => void
}

export interface PreviewSessionOptions {
  initialError: string | undefined
}

/**
 * Owns native startup subscriptions, loading and memory sampling.
 * `generation` rejects late native replies after a request is superseded.
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
  const [statusNotice, setStatusNotice] = useState<Notice | null>(null)
  const [viewportError, setViewportError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const [grid, setGrid] = useState(true)
  const [previewSettings, setPreviewSettings] = useState<PreviewSettings>(savedPreviewSettings)
  const [texturePacks, setTexturePacks] = useState<TexturePackState>({
    packs: [],
  })
  const [texturePackBusy, setTexturePackBusy] = useState(false)
  const [actionBusy, setActionBusy] = useState(false)
  const [choosing, setChoosing] = useState(false)
  const [imageAction, setImageAction] = useState<PreviewImageAction | null>(null)
  const viewportRef = useRef<HTMLDivElement>(null)
  const bootstrapRef = useRef<Bootstrap | null>(null)
  const viewportErrorRef = useRef<string | null>(null)
  const gridRef = useRef(true)
  const previewSettingsRef = useRef(previewSettings)
  const texturePacksRef = useRef(texturePacks)
  const texturePackBusyRef = useRef(false)
  const activePathRef = useRef<string | null>(null)
  const mounted = useRef(false)
  const generation = useRef(0)
  const initialPathConsumed = useRef(Boolean(initialError))
  const choosingRef = useRef(false)
  const actionBusyRef = useRef(false)
  const imageBusyRef = useRef(false)
  const titleQueue = useRef<Promise<void> | null>(null)
  const memoryRequestRef = useRef<number | null>(null)
  const sampleMemoryRef = useRef<(() => void) | null>(null)
  const listenersReady = useRef<Promise<void> | null>(null)
  // Only the current request may consume native progress and completion events.
  const pendingRequest = useRef<{
    id: number
    path: string
    started: number
  } | null>(null)

  // Persistent viewport failures remain visible across per-load notice resets.
  // Status notices report texture pack actions without closing the preview.
  const notice = useMemo(
    () =>
      viewportError !== null
        ? ({ intent: "error", message: viewportError } as const)
        : (rawNotice ?? statusNotice),
    [viewportError, rawNotice, statusNotice],
  )

  const updatePreviewSettings = useCallback((patch: Partial<PreviewSettings>) => {
    const settings = { ...previewSettingsRef.current, ...patch }
    previewSettingsRef.current = settings
    setPreviewSettings(settings)
  }, [])

  const isCurrent = useCallback((id: number) => mounted.current && generation.current === id, [])

  // Serialize title updates to prevent delayed calls restoring stale names.
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
  const failViewport = useCallback(
    (message: string) => {
      viewportQueue.setReady(false)
      viewportErrorRef.current = message
      setViewportError(message)
    },
    [viewportQueue],
  )
  useEffect(() => () => viewportQueue.clear(), [viewportQueue])
  const viewportInput = useCallback(
    (command: ViewportCommand) => viewportQueue.push(command),
    [viewportQueue],
  )

  const clearView = useCallback(() => {
    viewportQueue.resetScene()
    pendingRequest.current = null
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
  }, [viewportQueue])

  const home = useCallback(() => {
    const id = ++generation.current
    activePathRef.current = null
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

  // Raw errors open recovery; durable viewport failures stay in the notice bar.
  useEffect(() => {
    if (rawNotice?.intent !== "error") return
    setErrorDetails(rawNotice.message)
    setDialog("error")
    setDialogOpen(true)
    const id = ++generation.current
    activePathRef.current = null
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

  // Route uncaught failures through the notice bar.
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

  // Register every lifecycle listener before allowing the first native load.
  useEffect(() => {
    let active = true
    const unlisteners: Array<() => void> = []
    const track = (dispose: () => void) => {
      if (active) unlisteners.push(dispose)
      else dispose()
    }
    const ready = Promise.all([
      listen<PreviewProgress>("preview-progress", ({ payload }) => {
        const request = pendingRequest.current
        if (
          !active ||
          !request ||
          request.id !== payload.requestId ||
          !isCurrent(payload.requestId)
        )
          return
        if (payload.phase !== "decode" && payload.phase !== "mesh" && payload.phase !== "upload")
          return
        if (!Number.isSafeInteger(payload.total) || payload.total < 0) return
        if (
          !Number.isSafeInteger(payload.completed) ||
          payload.completed < 0 ||
          payload.completed > payload.total
        )
          return
        setLoading((previous) =>
          previous?.requestId === payload.requestId
            ? {
                ...previous,
                phase: payload.phase,
                completed: payload.completed,
                total: payload.total,
              }
            : previous,
        )
      }).then(track),
      listen<PreviewLoaded>("preview-loaded", ({ payload }) => {
        const request = pendingRequest.current
        if (
          !active ||
          !request ||
          request.id !== payload.requestId ||
          !isCurrent(payload.requestId)
        )
          return
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
      }).then(track),
      listen<PreviewFailed>("preview-failed", ({ payload }) => {
        const request = pendingRequest.current
        if (
          !active ||
          !request ||
          request.id !== payload.requestId ||
          !isCurrent(payload.requestId)
        )
          return
        pendingRequest.current = null
        setLoading(null)
        setPreviewMemory(null)
        setRawNotice({ intent: "error", message: `${request.path}\n\n${payload.message}` })
      }).then(track),
      listen<ViewportFailed>("viewport-failed", ({ payload }) => {
        if (active) failViewport(payload.message)
      }).then(track),
    ]).then(() => {})
    listenersReady.current = ready
    void ready.catch((error: unknown) => {
      if (active) failViewport(`Could not subscribe to preview events: ${errorMessage(error)}`)
    })
    return () => {
      active = false
      if (listenersReady.current === ready) listenersReady.current = null
      for (const dispose of unlisteners) dispose()
    }
  }, [failViewport, isCurrent, updateTitle])

  const loadPath = useCallback(
    async (path: string) => {
      if (!mounted.current) return
      // Each request snapshots settings, even if they change during decoding.
      const settings = previewSettingsRef.current
      const options = {
        chunkSize: settings.chunkingEnabled ? settings.chunkSize : null,
        threadCount:
          settings.chunkingEnabled && settings.multithreadingEnabled ? settings.threadCount : null,
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
      activePathRef.current = path
      setLoading({ path, requestId: id, phase: "decode", completed: 0, total: 0 })
      pendingRequest.current = {
        id,
        path,
        started,
      }
      try {
        await listenersReady.current
        if (!isCurrent(id)) return
        await invoke("start_preview", {
          path,
          requestId: id,
          options,
        })
      } catch (error) {
        if (!isCurrent(id) || pendingRequest.current?.id !== id) return
        pendingRequest.current = null
        setLoading(null)
        setPreviewMemory(null)
        if (errorMessage(error) !== "Cancelled")
          setRawNotice({ intent: "error", message: `${path}\n\n${errorMessage(error)}` })
      }
    },
    [cancelNative, clearView, isCurrent, updateTitle],
  )

  const reloadActivePreview = useCallback(() => {
    const path = activePathRef.current
    if (path) void loadPath(path)
  }, [loadPath])

  const beginTexturePackAction = useCallback(() => {
    if (texturePackBusyRef.current) return false
    texturePackBusyRef.current = true
    setTexturePackBusy(true)
    return true
  }, [])

  const endTexturePackAction = useCallback(() => {
    texturePackBusyRef.current = false
    if (mounted.current) setTexturePackBusy(false)
  }, [])

  const importTexturePacks = useCallback(async () => {
    if (!beginTexturePackAction()) return
    try {
      const result = await invoke<ImportedTexturePacks | null>("import_texture_packs")
      if (!mounted.current || !result) return
      texturePacksRef.current = result.state
      setTexturePacks(result.state)
      if (result.imported > 0) reloadActivePreview()
      if (result.failures.length > 0) {
        const added = result.imported > 0 ? `Added ${result.imported}. ` : ""
        setStatusNotice({
          intent: "error",
          message: `${added}Some texture packs could not be added: ${result.failures.join("; ")}`,
        })
      } else if (result.imported > 0) {
        setStatusNotice({
          intent: "success",
          message: `Added ${result.imported} texture pack${result.imported === 1 ? "" : "s"}.`,
        })
      }
    } catch (error) {
      if (mounted.current) setStatusNotice({ intent: "error", message: errorMessage(error) })
    } finally {
      endTexturePackAction()
    }
  }, [beginTexturePackAction, endTexturePackAction, reloadActivePreview])

  const setTexturePackEnabled = useCallback(
    async (id: string, enabled: boolean) => {
      const current = texturePacksRef.current.packs.find((pack) => pack.id === id)
      if (current?.enabled === enabled || !beginTexturePackAction()) return
      try {
        const next = await invoke<TexturePackState>("set_texture_pack_enabled", { id, enabled })
        if (!mounted.current) return
        texturePacksRef.current = next
        setTexturePacks(next)
        reloadActivePreview()
      } catch (error) {
        if (mounted.current) setStatusNotice({ intent: "error", message: errorMessage(error) })
      } finally {
        endTexturePackAction()
      }
    },
    [beginTexturePackAction, endTexturePackAction, reloadActivePreview],
  )

  const setTexturePackOrder = useCallback(
    async (ids: string[]) => {
      if (!beginTexturePackAction()) return
      try {
        const next = await invoke<TexturePackState>("set_texture_pack_order", { ids })
        if (!mounted.current) return
        texturePacksRef.current = next
        setTexturePacks(next)
        reloadActivePreview()
      } catch (error) {
        if (mounted.current) setStatusNotice({ intent: "error", message: errorMessage(error) })
      } finally {
        endTexturePackAction()
      }
    },
    [beginTexturePackAction, endTexturePackAction, reloadActivePreview],
  )

  const removeTexturePack = useCallback(
    async (id: string) => {
      if (!beginTexturePackAction()) return
      const wasEnabled = texturePacksRef.current.packs.find((pack) => pack.id === id)?.enabled
      try {
        const next = await invoke<TexturePackState>("remove_texture_pack", { id })
        if (!mounted.current) return
        texturePacksRef.current = next
        setTexturePacks(next)
        if (wasEnabled) reloadActivePreview()
      } catch (error) {
        if (mounted.current) setStatusNotice({ intent: "error", message: errorMessage(error) })
      } finally {
        endTexturePackAction()
      }
    },
    [beginTexturePackAction, endTexturePackAction, reloadActivePreview],
  )

  const requestImportTexturePacks = useCallback(() => {
    void importTexturePacks()
  }, [importTexturePacks])

  const requestSetTexturePackEnabled = useCallback(
    (id: string, enabled: boolean) => {
      void setTexturePackEnabled(id, enabled)
    },
    [setTexturePackEnabled],
  )

  const requestSetTexturePackOrder = useCallback(
    (ids: string[]) => {
      void setTexturePackOrder(ids)
    },
    [setTexturePackOrder],
  )

  const requestRemoveTexturePack = useCallback(
    (id: string) => {
      void removeTexturePack(id)
    },
    [removeTexturePack],
  )

  const chooseFile = useCallback(async () => {
    if (!mounted.current || !bootstrapRef.current || choosingRef.current) return
    choosingRef.current = true
    setChoosing(true)
    const id = generation.current
    // Canceling the picker preserves the active preview or load.
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

  const runImageAction = useCallback(
    async (action: PreviewImageAction) => {
      if (imageBusyRef.current) return
      imageBusyRef.current = true
      setImageAction(action)
      const id = generation.current
      try {
        if (action === "copy") {
          await invoke("copy_preview_image")
          if (isCurrent(id))
            setStatusNotice({ intent: "success", message: "Preview copied to the clipboard." })
        } else {
          // The native command returns false when the save dialog is canceled.
          const saved = await invoke<boolean>("export_preview_image")
          if (saved && isCurrent(id))
            setStatusNotice({ intent: "success", message: "Preview image saved." })
        }
      } catch (error) {
        if (isCurrent(id))
          setStatusNotice({
            intent: "error",
            message:
              action === "copy"
                ? `Could not copy the preview image: ${errorMessage(error)}`
                : `Could not export the preview image: ${errorMessage(error)}`,
          })
      } finally {
        imageBusyRef.current = false
        if (mounted.current) setImageAction(null)
      }
    },
    [isCurrent],
  )

  const copyImage = useCallback(() => {
    if (loaded) void runImageAction("copy")
  }, [loaded, runImageAction])

  const exportImage = useCallback(() => {
    if (loaded) void runImageAction("export")
  }, [loaded, runImageAction])

  useEffect(() => {
    mounted.current = true
    let active = true
    let unlisten: (() => void) | undefined
    const initialGeneration = generation.current
    void (listenersReady.current ?? Promise.resolve())
      .then(() => {
        if (!active) return null
        return invoke<Bootstrap>("bootstrap")
      })
      .then((result) => {
        if (!active || !result) return
        const openInitialPath = isCurrent(initialGeneration)
        generation.current = Math.max(generation.current, result.requestId)
        bootstrapRef.current = result
        setBootstrap(result)
        texturePacksRef.current = result.texturePacks
        setTexturePacks(result.texturePacks)
        if (result.texturePackError)
          setStatusNotice({ intent: "info", message: result.texturePackError })
        // Bootstrap replays viewport failures that occurred before listeners were installed.
        if (result.viewportError) failViewport(result.viewportError)
        viewportQueue.push({ kind: "grid", visible: gridRef.current })
        viewportQueue.setReady(viewportErrorRef.current === null)
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
          if (result.initialPath && openInitialPath && viewportErrorRef.current === null)
            void loadPath(result.initialPath)
        }
      })
      .catch((error: unknown) => {
        if (active) failViewport(`Could not initialize the application: ${errorMessage(error)}`)
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
  }, [cancelNative, failViewport, isCurrent, loadPath, updatePreviewSettings, viewportQueue])

  // One session-long subscription reads the active request id from a ref.
  useEffect(() => {
    let inFlight = false
    const sample = async () => {
      const id = memoryRequestRef.current
      if (id === null || inFlight) return
      inFlight = true
      try {
        const memory = await invoke<number | null>("preview_memory", { requestId: id })
        if (memoryRequestRef.current === id && isCurrent(id)) setPreviewMemory(memory)
      } catch {
        if (memoryRequestRef.current === id && isCurrent(id)) setPreviewMemory(null)
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
  }, [isCurrent])

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

  const clearNotice = useCallback((_notice: Notice | null) => {
    setRawNotice(null)
    setStatusNotice(null)
  }, [])

  return {
    actionBusy,
    bootstrap,
    busy: !bootstrap || choosing,
    choosing,
    copyImage,
    dialog,
    dialogOpen,
    dragging,
    errorDetails,
    exportImage,
    fit,
    grid,
    home,
    imageAction,
    importTexturePacks: requestImportTexturePacks,
    loadDemo,
    loaded,
    loading,
    nativeAction: runNativeAction,
    notice,
    openDialog,
    openFile,
    previewMemory,
    previewSettings,
    removeTexturePack: requestRemoveTexturePack,
    setDialogOpen,
    setNotice: clearNotice,
    setTexturePackEnabled: requestSetTexturePackEnabled,
    setTexturePackOrder: requestSetTexturePackOrder,
    texturePackBusy,
    texturePacks,
    toggleGrid,
    updatePreviewSettings,
    viewportInput,
    viewportRef,
    zoomIn,
    zoomOut,
  } satisfies PreviewSession
}
