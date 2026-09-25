import { useCallback, useEffect, useRef, useState } from "react"
import {
  Badge,
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  FluentProvider,
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  ProgressBar,
  Slider,
  SpinButton,
  Switch,
  ToggleButton,
  Tooltip,
  webDarkTheme,
  webLightTheme,
} from "@fluentui/react-components"
import {
  Add20Regular,
  ArrowExpand20Regular,
  ArrowRight20Regular,
  Cube24Regular,
  Dismiss20Regular,
  Document20Regular,
  FolderOpen20Regular,
  Grid20Regular,
  Home20Regular,
  Info20Regular,
  Keyboard20Regular,
  MoreHorizontal20Regular,
  Settings20Regular,
  ShieldCheckmark20Regular,
  Subtract20Regular,
} from "@fluentui/react-icons"
import { listen } from "@tauri-apps/api/event"
import { invoke } from "@tauri-apps/api/core"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { getCurrentWindow } from "@tauri-apps/api/window"
import icon from "../../Assets/app-ui.png"
import { SchematicRenderer, type PreviewMetadata, type PreviewStreamEvent } from "./renderer"
import type { PreviewReadRange } from "./upload-layout"

type Bootstrap = {
  extensions: string[]
  demos: { name: string; path: string; extension: string }[]
  initialPath: string | null
  version: string
  requestId: number
  maxWorkerThreads: number
}
type ThemePreference = "system" | "light" | "dark"
type PreviewSettings = {
  memoryLimitEnabled: boolean
  memoryLimitMB: number
  chunkingEnabled: boolean
  chunkSize: number
  multithreadingEnabled: boolean
  threadCount: number
  conservativeMemoryScheduling: boolean
}
type Loading = {
  path: string
  requestId: number
  phase: "decode" | "mesh" | "upload" | "stream"
  completed: number
  total: number
  uploadedBytes: number
}
type MeshProgress = { requestId: number; phase: "mesh"; completed: number; total: number }
type Loaded = { path: string; metadata: PreviewMetadata; seconds: number }
type Notice = { intent: "error" | "success" | "info"; message: string }

const appName = "Litematica Preview"
const numbers = new Intl.NumberFormat()
const dimensions = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 2,
})
const megabytes = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 })
const formatMB = (bytes: number) => `${megabytes.format(bytes / (1024 * 1024))} MB`
const chunkSizes = [16, 32, 64, 128, 256]
const defaultPreviewSettings: PreviewSettings = {
  memoryLimitEnabled: false,
  memoryLimitMB: 2048,
  chunkingEnabled: true,
  chunkSize: 64,
  multithreadingEnabled: true,
  threadCount: 4,
  conservativeMemoryScheduling: false,
}
const fileName = (path: string) => path.split(/[\\/]/).pop() || path
const errorMessage = (error: unknown) => (error instanceof Error ? error.message : String(error))
const isTheme = (value: unknown): value is ThemePreference =>
  value === "system" || value === "light" || value === "dark"

function savedTheme(): ThemePreference {
  try {
    const value = localStorage.getItem("litematica-preview-theme")
    return isTheme(value) ? value : "system"
  } catch {
    return "system"
  }
}

export function savedPreviewSettings(): PreviewSettings {
  try {
    const value: unknown = JSON.parse(localStorage.getItem("litematica-preview-settings") || "null")
    if (value === null || typeof value !== "object" || Array.isArray(value))
      return defaultPreviewSettings
    const settings = value as Record<string, unknown>
    return {
      memoryLimitEnabled:
        typeof settings.memoryLimitEnabled === "boolean"
          ? settings.memoryLimitEnabled
          : defaultPreviewSettings.memoryLimitEnabled,
      memoryLimitMB:
        typeof settings.memoryLimitMB === "number" &&
        Number.isInteger(settings.memoryLimitMB) &&
        settings.memoryLimitMB >= 2048 &&
        settings.memoryLimitMB <= 8192
          ? settings.memoryLimitMB
          : typeof settings.memoryLimitGiB === "number" &&
              Number.isInteger(settings.memoryLimitGiB) &&
              settings.memoryLimitGiB >= 2 &&
              settings.memoryLimitGiB <= 8
            ? settings.memoryLimitGiB * 1024
            : defaultPreviewSettings.memoryLimitMB,
      chunkingEnabled:
        typeof settings.chunkingEnabled === "boolean"
          ? settings.chunkingEnabled
          : defaultPreviewSettings.chunkingEnabled,
      chunkSize:
        typeof settings.chunkSize === "number" && chunkSizes.includes(settings.chunkSize)
          ? settings.chunkSize
          : defaultPreviewSettings.chunkSize,
      multithreadingEnabled:
        settings.multithreadingEnabled === true && settings.chunkingEnabled !== false,
      threadCount:
        typeof settings.threadCount === "number" &&
        Number.isInteger(settings.threadCount) &&
        settings.threadCount >= 2 &&
        settings.threadCount <= 8
          ? settings.threadCount
          : defaultPreviewSettings.threadCount,
      conservativeMemoryScheduling:
        typeof settings.conservativeMemoryScheduling === "boolean"
          ? settings.conservativeMemoryScheduling
          : typeof settings.speedFirst === "boolean"
            ? !settings.speedFirst
            : defaultPreviewSettings.conservativeMemoryScheduling,
    }
  } catch {
    return defaultPreviewSettings
  }
}

export default function App({ initialError }: { initialError?: string }) {
  const [bootstrap, setBootstrap] = useState<Bootstrap | null>(null)
  const [loading, setLoading] = useState<Loading | null>(null)
  const [previewMemory, setPreviewMemory] = useState<number | null>(null)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [notice, setNotice] = useState<Notice | null>(null)
  const [dragging, setDragging] = useState(false)
  const [grid, setGrid] = useState(true)
  const [theme, setTheme] = useState<ThemePreference>(savedTheme)
  const [previewSettings, setPreviewSettings] = useState<PreviewSettings>(savedPreviewSettings)
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  )
  const [dialog, setDialog] = useState<"controls" | "about" | "settings" | "error">(
    initialError ? "error" : "controls",
  )
  const [dialogOpen, setDialogOpen] = useState(Boolean(initialError))
  const [errorDetails, setErrorDetails] = useState(initialError || "")
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
  const titleQueue = useRef<Promise<void>>(Promise.resolve())
  const previewReadQueue = useRef<Promise<void>>(Promise.resolve())

  const updatePreviewSettings = useCallback((patch: Partial<PreviewSettings>) => {
    const settings = { ...previewSettingsRef.current, ...patch }
    previewSettingsRef.current = settings
    setPreviewSettings(settings)
  }, [])

  const isCurrent = useCallback((id: number) => mounted.current && generation.current === id, [])

  // Serialize title changes so a delayed native call cannot restore an old filename.
  const updateTitle = useCallback(
    (path: string | null, id: number) => {
      titleQueue.current = titleQueue.current.then(async () => {
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

  useEffect(() => {
    if (notice?.intent !== "error") return
    const id = ++generation.current
    const renderer = rendererRef.current
    rendererRef.current = null
    setLoaded(null)
    setLoading(null)
    setPreviewMemory(null)
    setDragging(false)
    setNotice(null)
    setErrorDetails(notice.message)
    setDialog("error")
    setDialogOpen(true)
    // Recovery failures are included in the same dialog, never recursively reported.
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
      setLoading({ path, requestId: id, phase: "decode", completed: 0, total: 0, uploadedBytes: 0 })
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
        // Serial mode completes native generation before allocating GPU state.
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
              const read = previewReadQueue.current.then(() => readRanges(null, ranges))
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
    // Do not replace the active preview if another generation starts while the picker is open.
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

  const nativeAction = useCallback(
    async (command: "register_associations" | "unregister_associations" | "show_licenses") => {
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
        if (mounted.current) setActionBusy(false)
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

  useEffect(() => {
    if (!loading) return
    const id = loading.requestId
    let active = true
    let inFlight = false
    const sample = async () => {
      if (inFlight) return
      inFlight = true
      try {
        const memory = await invoke<number | null>("preview_memory", { requestId: id })
        if (active && isCurrent(id)) setPreviewMemory(memory)
      } catch {
        if (active && isCurrent(id)) setPreviewMemory(null)
      } finally {
        inFlight = false
      }
    }
    void sample()
    const timer = window.setInterval(() => void sample(), 500)
    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [loading?.requestId, isCurrent])

  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)")
    const change = () => setSystemDark(media.matches)
    media.addEventListener("change", change)
    return () => media.removeEventListener("change", change)
  }, [])

  const dark = theme === "dark" || (theme === "system" && systemDark)
  // Storage failures do not change in-memory preferences.
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark)
    document.documentElement.style.colorScheme = dark ? "dark" : "light"
    try {
      localStorage.setItem("litematica-preview-theme", theme)
    } catch {}
  }, [dark, theme])

  useEffect(() => {
    try {
      localStorage.setItem("litematica-preview-settings", JSON.stringify(previewSettings))
    } catch {}
  }, [previewSettings])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey) return
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "o") {
        event.preventDefault()
        if (!dialogOpen) void chooseFile()
        return
      }
      if (dialogOpen || event.ctrlKey || event.metaKey) return
      const target = event.target
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target.closest('input, textarea, select, [role="menu"], [role="dialog"]'))
      )
        return
      if (event.key === "Escape" && loading) {
        event.preventDefault()
        home()
      } else if (event.key.toLowerCase() === "f" && loaded && target !== canvasRef.current) {
        event.preventDefault()
        rendererRef.current?.fit()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [chooseFile, dialogOpen, home, loaded, loading])

  const toggleGrid = () => {
    const visible = !gridRef.current
    gridRef.current = visible
    setGrid(visible)
    rendererRef.current?.setGrid(visible)
  }
  const stageActive = Boolean(loaded || loading)
  const size = loaded?.metadata.max
    .map((maximum, axis) => dimensions.format(maximum - loaded.metadata.min[axis]))
    .join(" × ")

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      // Portals inherit theme tokens, not the full-window app-shell layout.
      applyStylesToPortals={false}
      className={`flex flex-col w-full h-full min-w-[320px] text-text bg-surface-secondary ${dark ? "dark theme-dark" : "theme-light"}`}
    >
      <header className="flex-none flex items-center justify-between gap-4 min-h-14 sm:min-h-16 px-4 py-2.5 sm:px-6 sm:py-3 bg-surface border-b border-border">
        <div className="flex items-center gap-2.5 sm:gap-3 text-sm sm:text-base font-semibold tracking-tight">
          <img className="object-contain flex-none" src={icon} width="30" height="30" alt="" />
          <span>{appName}</span>
          <Badge
            appearance="outline"
            className="ml-1.5! font-normal! text-muted! hidden! sm:inline-flex!"
          >
            Desktop
          </Badge>
        </div>
        <Menu
          checkedValues={{ theme: [theme] }}
          onCheckedValueChange={(_, data) => {
            const value = data.checkedItems[0]
            if (data.name === "theme" && isTheme(value)) setTheme(value)
          }}
        >
          <MenuTrigger disableButtonEnhancement>
            <Button
              appearance="subtle"
              icon={<MoreHorizontal20Regular />}
              aria-label="Application menu"
              title="Application menu"
            />
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem
                icon={<Settings20Regular />}
                onClick={() => {
                  setDialog("settings")
                  setDialogOpen(true)
                }}
              >
                Preview settings
              </MenuItem>
              <MenuItem
                icon={<Keyboard20Regular />}
                onClick={() => {
                  setDialog("controls")
                  setDialogOpen(true)
                }}
              >
                Controls and shortcuts
              </MenuItem>
              <MenuItem
                icon={<Info20Regular />}
                onClick={() => {
                  setDialog("about")
                  setDialogOpen(true)
                }}
              >
                About and licenses
              </MenuItem>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>File associations</MenuGroupHeader>
                <MenuItem
                  disabled={actionBusy}
                  onClick={() => void nativeAction("register_associations")}
                >
                  Set as default app…
                </MenuItem>
                <MenuItem
                  disabled={actionBusy}
                  onClick={() => void nativeAction("unregister_associations")}
                >
                  Remove file associations
                </MenuItem>
              </MenuGroup>
              <MenuDivider />
              <MenuGroup>
                <MenuGroupHeader>Appearance</MenuGroupHeader>
                <MenuItemRadio name="theme" value="system">
                  Use system setting
                </MenuItemRadio>
                <MenuItemRadio name="theme" value="light">
                  Light
                </MenuItemRadio>
                <MenuItemRadio name="theme" value="dark">
                  Dark
                </MenuItemRadio>
              </MenuGroup>
            </MenuList>
          </MenuPopover>
        </Menu>
      </header>

      <nav
        className="flex-none flex items-center gap-1.5 sm:gap-2 min-h-14 px-3 py-2 sm:px-5 sm:py-2.5 border-b border-border bg-surface [&>button]:h-9! [&>button]:shrink-0 [&>button:has(span.hidden)]:px-3! [&>button:has(span.hidden)]:gap-2! [&>button:not(:has(span.hidden))]:w-9! [&>button:not(:has(span.hidden))]:min-w-9!"
        aria-label="Preview commands"
      >
        <Button
          appearance="primary"
          icon={<FolderOpen20Regular />}
          disabled={!bootstrap || choosing}
          onClick={() => void chooseFile()}
          title="Open schematic (Ctrl+O)"
        >
          Open<span className="ml-1 opacity-75 text-xs font-normal hidden sm:inline">Ctrl+O</span>
        </Button>
        <Tooltip content="Return home" relationship="label">
          <Button appearance="subtle" icon={<Home20Regular />} onClick={home} aria-label="Home" />
        </Tooltip>
        <span className="self-center h-5 w-px mx-0.5 sm:mx-1 bg-border shrink-0" />
        <Button
          appearance="subtle"
          icon={<ArrowExpand20Regular />}
          disabled={!loaded}
          onClick={() => rendererRef.current?.fit()}
          title="Fit schematic (F)"
        >
          Fit<span className="ml-1 opacity-75 text-xs font-normal hidden sm:inline">F</span>
        </Button>
        <Tooltip content="Zoom out (−)" relationship="label">
          <Button
            appearance="subtle"
            icon={<Subtract20Regular />}
            disabled={!loaded}
            onClick={() => rendererRef.current?.zoom(1.12)}
            aria-label="Zoom out"
          />
        </Tooltip>
        <Tooltip content="Zoom in (+)" relationship="label">
          <Button
            appearance="subtle"
            icon={<Add20Regular />}
            disabled={!loaded}
            onClick={() => rendererRef.current?.zoom(0.88)}
            aria-label="Zoom in"
          />
        </Tooltip>
        <span className="self-center h-5 w-px mx-0.5 sm:mx-1 bg-border shrink-0" />
        <Tooltip content={grid ? "Hide ground grid" : "Show ground grid"} relationship="label">
          <ToggleButton
            appearance="subtle"
            icon={<Grid20Regular />}
            checked={grid}
            onClick={toggleGrid}
            aria-label="Ground grid"
          />
        </Tooltip>
        <span
          className="ml-auto pl-4 max-w-[40%] md:max-w-[30%] hidden sm:block text-muted text-xs whitespace-nowrap overflow-hidden text-ellipsis"
          title={loaded?.path || loading?.path}
        >
          {loaded ? fileName(loaded.path) : loading ? fileName(loading.path) : "Ready when you are"}
        </span>
      </nav>

      {notice && (
        <MessageBar intent={notice.intent} className="flex-none rounded-none! break-words">
          <MessageBarBody>
            <MessageBarTitle>
              {notice.intent === "error"
                ? "Something went wrong"
                : notice.intent === "success"
                  ? "Done"
                  : "Please note"}
            </MessageBarTitle>
            {notice.message}
          </MessageBarBody>
          <MessageBarActions
            containerAction={
              <Button
                appearance="transparent"
                icon={<Dismiss20Regular />}
                aria-label="Dismiss message"
                onClick={() => setNotice(null)}
              />
            }
          />
        </MessageBar>
      )}

      <main
        className="relative flex-auto min-h-0 overflow-hidden"
        aria-label={stageActive ? "Schematic preview" : "Welcome"}
      >
        <section
          className="absolute inset-0 overflow-auto [overscroll-behavior:contain]"
          hidden={stageActive}
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
                disabled={!bootstrap || choosing}
                onClick={() => void chooseFile()}
                className="w-full sm:w-auto!"
              >
                {choosing ? "Choosing file…" : "Open schematic"}
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
                      onClick={() => void loadPath(demo.path)}
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
          {loading && (
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
                <p className="mt-0 mb-3 text-sm leading-relaxed text-muted">
                  {loading.phase === "decode"
                    ? "Reading blocks locally."
                    : loading.phase === "mesh" || loading.phase === "stream"
                      ? loading.total > 0
                        ? `Generating geometry: ${numbers.format(loading.completed)} / ${numbers.format(loading.total)} chunks (${Math.round((loading.completed / loading.total) * 100)}%).`
                        : "Generating geometry and uploading ready batches."
                      : `Uploading geometry and textures: ${formatMB(loading.completed)} / ${formatMB(loading.total)} (${Math.round((loading.completed / loading.total) * 100)}%).`}
                </p>
                {loading.phase === "stream" && (
                  <p className="mt-0 mb-3 text-sm leading-relaxed text-muted">
                    Uploaded {formatMB(loading.uploadedBytes)} model data. Generation and upload
                    overlap; the preview appears only when both finish.
                  </p>
                )}
                <ProgressBar
                  value={
                    loading.phase === "decode" || loading.total === 0
                      ? undefined
                      : loading.completed
                  }
                  max={
                    loading.phase === "decode" || loading.total === 0 ? undefined : loading.total
                  }
                  aria-label={
                    loading.phase === "decode"
                      ? "Reading blocks"
                      : loading.phase === "mesh" || loading.phase === "stream"
                        ? "Generating geometry"
                        : "Uploading model data"
                  }
                  className="mb-4"
                />
                <Button appearance="secondary" onClick={home} className="mt-1.5!">
                  Cancel
                  <span className="ml-2.5 opacity-75 text-xs font-normal hidden sm:inline">
                    Esc
                  </span>
                </Button>
              </div>
            </div>
          )}
        </section>

        {dragging && (
          <div
            className="absolute z-10 inset-3 sm:inset-4 grid place-items-center border-2 border-dashed border-accent rounded-xl bg-surface opacity-95 text-center pointer-events-none"
            role="status"
          >
            <div className="[&>svg]:w-10 [&>svg]:h-10 [&>svg]:mb-4 [&>svg]:text-accent">
              <FolderOpen20Regular />
              <h2 className="mt-0 mb-2 text-2xl font-semibold">Drop to preview</h2>
              <p className="mt-0 px-4 text-muted text-sm">
                The first supported schematic will open.
              </p>
            </div>
          </div>
        )}
      </main>

      <footer
        className="flex-none flex items-center flex-wrap gap-x-4 sm:gap-x-6 gap-y-1.5 min-h-9 px-4 sm:px-6 py-2 border-t border-border bg-surface text-muted text-xs leading-normal [&_strong]:text-text [&_strong]:font-semibold"
        aria-label="Preview information"
      >
        {loaded ? (
          <>
            <span>
              <strong>{numbers.format(loaded.metadata.blockCount)}</strong> blocks
            </span>
            <span title="Geometry dimensions in blocks">{size} blocks</span>
            <span>{numbers.format(loaded.metadata.triangleCount)} triangles</span>
            <span className="ml-auto">
              {formatMB(loaded.metadata.byteLength)} model data loaded in{" "}
              {loaded.seconds.toFixed(2)} s.
            </span>
          </>
        ) : (
          <>
            <span role="status" className="flex items-center gap-2">
              {!loading && !choosing && <ShieldCheckmark20Regular className="shrink-0" />}
              {loading
                ? loading.phase === "decode"
                  ? "Decoding…"
                  : loading.phase === "mesh"
                    ? "Generating geometry…"
                    : loading.phase === "stream"
                      ? "Generating and uploading…"
                      : "Uploading to graphics device…"
                : choosing
                  ? "Choose a schematic in the file dialog"
                  : "Everything works offline."}
            </span>
            {loading && (
              <div className="ml-auto flex max-w-full flex-wrap justify-end gap-x-3 gap-y-1 text-right">
                {previewMemory !== null ? (
                  <span title="Host and decoder private working sets (resident private pages only). WebView2 and GPU memory are excluded.">
                    Process memory {formatMB(previewMemory)}
                  </span>
                ) : (
                  <span>Sampling memory…</span>
                )}
                {(loading.phase === "upload" || loading.phase === "stream") && (
                  <span>{formatMB(loading.uploadedBytes)} model data uploaded</span>
                )}
              </div>
            )}
            {!loading && bootstrap && <span className="ml-auto">v{bootstrap.version}</span>}
          </>
        )}
      </footer>

      <Dialog
        open={dialogOpen}
        onOpenChange={(_, data) => {
          setDialogOpen(data.open)
        }}
      >
        <DialogSurface>
          <DialogBody>
            <DialogTitle>
              {dialog === "error"
                ? "Unable to open preview"
                : dialog === "controls"
                  ? "Controls and shortcuts"
                  : dialog === "settings"
                    ? "Preview settings"
                    : `About ${appName}`}
            </DialogTitle>
            <DialogContent>
              {dialog === "error" ? (
                <>
                  <p>The preview was closed. You are back on the home screen.</p>
                  <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] max-h-[45vh] overflow-auto select-text text-xs">
                    {errorDetails}
                  </pre>
                </>
              ) : dialog === "settings" ? (
                <div className="flex flex-col gap-5 max-h-[60vh] overflow-y-auto">
                  <p className="m-0 leading-relaxed">
                    Changes are saved automatically and apply the next time you open a schematic.
                    The current preview or load is not changed.
                  </p>
                  <div className="flex flex-col gap-3">
                    <div className="flex items-center justify-between gap-4">
                      <Switch
                        label="Limit decoder memory"
                        checked={previewSettings.memoryLimitEnabled}
                        aria-describedby="memory-limit-description"
                        onChange={(_, data) =>
                          updatePreviewSettings({ memoryLimitEnabled: data.checked })
                        }
                      />
                      <div className="ml-auto flex items-center gap-2">
                        <SpinButton
                          value={previewSettings.memoryLimitMB}
                          min={2048}
                          max={8192}
                          step={1}
                          disabled={!previewSettings.memoryLimitEnabled}
                          aria-label="Decoder memory limit (MB)"
                          aria-describedby="memory-limit-description"
                          className="w-32"
                          onChange={(_, data) => {
                            const value =
                              data.value ??
                              (data.displayValue && /^\d+$/.test(data.displayValue)
                                ? Number(data.displayValue)
                                : null)
                            if (
                              value !== null &&
                              Number.isInteger(value) &&
                              value >= 2048 &&
                              value <= 8192
                            )
                              updatePreviewSettings({ memoryLimitMB: value })
                          }}
                        />
                        <span className="text-sm">MB</span>
                      </div>
                    </div>
                    <p
                      id="memory-limit-description"
                      className="m-0 text-sm leading-relaxed text-muted"
                    >
                      Limits the isolated decoder process, not graphics or total application memory.
                      If the decoder stops while this limit is enabled, loading returns to Home and
                      shows an error; the limit may be involved, but a crash cannot confirm it was
                      reached. Disabling the limit may exhaust system memory.
                    </p>
                  </div>
                  <div className="flex flex-col gap-3">
                    <Switch
                      label="Separate geometry into chunks"
                      checked={previewSettings.chunkingEnabled}
                      disabled={previewSettings.multithreadingEnabled}
                      aria-describedby="chunk-size-description"
                      onChange={(_, data) =>
                        updatePreviewSettings({ chunkingEnabled: data.checked })
                      }
                    />
                    <Field label={`Chunk size (blocks per side): ${previewSettings.chunkSize}`}>
                      <Slider
                        className="chunk-size-slider mb-8"
                        min={0}
                        max={chunkSizes.length - 1}
                        step={1}
                        aria-label="Chunk size (blocks per side)"
                        value={chunkSizes.indexOf(previewSettings.chunkSize)}
                        disabled={!previewSettings.chunkingEnabled}
                        rail={{
                          className: "chunk-size-rail",
                          children: chunkSizes.map((size, index) => (
                            <span
                              key={size}
                              aria-hidden="true"
                              className="chunk-size-mark"
                              style={{ left: `${(index / (chunkSizes.length - 1)) * 100}%` }}
                            >
                              <span className="chunk-size-mark-dot" />
                              <span className="chunk-size-mark-label">{size}</span>
                            </span>
                          )),
                        }}
                        onChange={(_, data) =>
                          updatePreviewSettings({ chunkSize: chunkSizes[data.value] })
                        }
                      />
                    </Field>
                    <p
                      id="chunk-size-description"
                      className="m-0 text-sm leading-relaxed text-muted"
                    >
                      Smaller chunks reduce peak meshing memory and allow cancellation between
                      chunks. Disabling chunk separation increases peak memory use and cancellation
                      latency.
                    </p>
                  </div>
                  <div className="flex flex-col gap-3">
                    <Switch
                      label="Enable multithreading"
                      checked={previewSettings.multithreadingEnabled}
                      disabled={
                        !previewSettings.chunkingEnabled ||
                        !bootstrap ||
                        bootstrap.maxWorkerThreads < 2
                      }
                      aria-describedby="thread-count-description"
                      onChange={(_, data) =>
                        updatePreviewSettings({ multithreadingEnabled: data.checked })
                      }
                    />
                    <Field label="Worker threads">
                      <SpinButton
                        value={previewSettings.threadCount}
                        min={2}
                        max={Math.max(2, bootstrap?.maxWorkerThreads ?? 2)}
                        step={1}
                        disabled={!previewSettings.multithreadingEnabled}
                        aria-label="Worker threads"
                        aria-describedby="thread-count-description"
                        className="w-32"
                        onChange={(_, data) => {
                          const value =
                            data.value ??
                            (data.displayValue && /^\d+$/.test(data.displayValue)
                              ? Number(data.displayValue)
                              : null)
                          if (
                            value !== null &&
                            Number.isInteger(value) &&
                            value >= 2 &&
                            value <= (bootstrap?.maxWorkerThreads ?? 1)
                          )
                            updatePreviewSettings({ threadCount: value })
                        }}
                      />
                    </Field>
                    <Switch
                      label="Conservative memory scheduling"
                      checked={previewSettings.conservativeMemoryScheduling}
                      disabled={!previewSettings.multithreadingEnabled}
                      aria-describedby="conservative-memory-description"
                      onChange={(_, data) =>
                        updatePreviewSettings({ conservativeMemoryScheduling: data.checked })
                      }
                    />
                    <p
                      id="conservative-memory-description"
                      className="m-0 text-sm leading-relaxed text-muted"
                    >
                      Disabled by default. Enable it to reduce concurrent mesh work and the number
                      of queued batches and upload pages; this may reduce decoding speed. Leaving it
                      disabled uses the selected worker count more aggressively and may use more
                      memory. A separate decoder memory limit remains effective in either mode.
                    </p>
                    <p
                      id="thread-count-description"
                      className="m-0 text-sm leading-relaxed text-muted"
                    >
                      Requires chunk separation and at least two available logical processors. Uses
                      up to {bootstrap?.maxWorkerThreads ?? 1} workers for parallel decoding,
                      meshing and upload preparation. Memory-first scheduling may use fewer workers;
                      additional working buffers can still increase peak memory. GPU submission
                      remains on the main thread.
                    </p>
                  </div>
                </div>
              ) : dialog === "controls" ? (
                <>
                  <p className="mt-0 mb-5 leading-relaxed">
                    Click or Tab into the preview to use its keyboard controls.
                  </p>
                  <dl className="flex flex-col gap-0 my-0 mb-5 [&>div]:grid [&>div]:grid-cols-[80px_1fr] sm:[&>div]:grid-cols-[120px_1fr] [&>div]:gap-3 sm:[&>div]:gap-4 [&>div]:py-3 [&>div]:border-b [&>div]:border-[var(--colorNeutralStroke2,#dddddd)] [&_dt]:font-semibold [&_dd]:m-0 [&_dd]:leading-normal">
                    <div>
                      <dt>Orbit</dt>
                      <dd>Left-drag / Arrow keys</dd>
                    </div>
                    <div>
                      <dt>Pan</dt>
                      <dd>Right- or middle-drag / Shift + Arrow keys</dd>
                    </div>
                    <div>
                      <dt>Zoom</dt>
                      <dd>
                        Scroll / <kbd>+</kbd> or <kbd>−</kbd>
                      </dd>
                    </div>
                    <div>
                      <dt>Fit model</dt>
                      <dd>
                        <kbd>F</kbd> / <kbd>Home</kbd> in the preview
                      </dd>
                    </div>
                    <div>
                      <dt>Open schematic</dt>
                      <dd>
                        <kbd>Ctrl</kbd> + <kbd>O</kbd>
                      </dd>
                    </div>
                    <div>
                      <dt>Cancel loading</dt>
                      <dd>
                        <kbd>Esc</kbd>
                      </dd>
                    </div>
                  </dl>
                  <p className="text-muted leading-relaxed">
                    Use the grid button to show or hide the ground grid. Home on the command bar
                    returns to your bundled examples.
                  </p>
                </>
              ) : (
                <div className="leading-relaxed [&>p]:mb-3">
                  <div className="flex items-center gap-3.5 my-3 mb-6 [&>div]:flex [&>div]:flex-col [&>div]:gap-1 [&_strong]:text-lg [&_strong]:font-semibold [&_span]:text-xs">
                    <img
                      className="object-contain flex-none"
                      src={icon}
                      width="48"
                      height="48"
                      alt=""
                    />
                    <div>
                      <strong>{appName}</strong>
                      <span>Version {bootstrap?.version || "unavailable"}</span>
                    </div>
                  </div>
                  <p>
                    A local, interactive viewer for Minecraft schematics and structures, inspired by
                    LitematicaQL.
                  </p>
                  <p>Powered by Nucleation, Tauri, WebGL, React, and Fluent UI.</p>
                  <p>
                    Distributed under the GNU Affero General Public License v3. This program comes
                    with no warranty. Redistribution is permitted under the terms of the bundled
                    license.
                  </p>
                  <p className="text-muted">
                    The application license and third-party notices are included with your
                    installation.
                  </p>
                </div>
              )}
            </DialogContent>
            <DialogActions>
              {dialog === "about" && (
                <Button
                  disabled={actionBusy}
                  onClick={() => {
                    setDialogOpen(false)
                    void nativeAction("show_licenses")
                  }}
                >
                  Open licenses folder
                </Button>
              )}
              <Button appearance="primary" onClick={() => setDialogOpen(false)}>
                Close
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </FluentProvider>
  )
}
