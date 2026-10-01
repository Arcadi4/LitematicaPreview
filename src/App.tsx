import { useEffect, useEffectEvent, useState } from "react"
import { FluentProvider, webDarkTheme, webLightTheme } from "@fluentui/react-components"
import { AppDialog } from "./app-dialog"
import { PreviewFooter } from "./app-footer"
import { AppHeader } from "./app-header"
import { isTheme, savedTheme, type ThemePreference } from "./app-model"
import { PreviewToolbar } from "./app-toolbar"
import { DropOverlay } from "./drop-overlay"
import { NoticeBar } from "./notice-bar"
import { PreviewStage } from "./preview-stage"
import { usePreviewSession } from "./use-preview-session"
import { WelcomePanel } from "./welcome-panel"

export default function App({ initialError }: { initialError?: string }) {
  const [theme, setTheme] = useState<ThemePreference>(savedTheme)
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  )

  const {
    actionBusy,
    bootstrap,
    busy,
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
    nativeAction,
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
  } = usePreviewSession({ initialError })

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
    } catch {
      /* Theme still works when storage is disabled. */
    }
  }, [dark, theme])

  useEffect(() => {
    try {
      localStorage.setItem("litematica-preview-settings", JSON.stringify(previewSettings))
    } catch {
      /* Preview settings still work when storage is disabled. */
    }
  }, [previewSettings])

  // Effect Events keep the keydown subscription stable while still seeing the
  // latest callbacks and state.
  const requestFile = useEffectEvent(() => openFile())
  const returnHome = useEffectEvent(() => home())

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey) return
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "o") {
        event.preventDefault()
        if (!dialogOpen) requestFile()
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
        returnHome()
      } else if (event.key.toLowerCase() === "f" && loaded && target !== canvasRef.current) {
        event.preventDefault()
        fit()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [canvasRef, dialogOpen, fit, loaded, loading])

  const onThemeChange = (value: unknown) => {
    if (isTheme(value)) setTheme(value)
  }
  const stageActive = Boolean(loaded || loading)

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      // Portals inherit theme tokens, not the full-window app-shell layout.
      applyStylesToPortals={false}
      className={`flex flex-col w-full h-full min-w-[320px] text-text bg-surface-secondary ${dark ? "dark theme-dark" : "theme-light"}`}
    >
      <AppHeader actionBusy={actionBusy} onNativeAction={nativeAction} onOpenDialog={openDialog} />

      <PreviewToolbar
        busy={busy}
        grid={grid}
        loaded={loaded}
        loading={loading}
        onFit={fit}
        onHome={home}
        onOpenFile={openFile}
        onToggleGrid={toggleGrid}
        onZoomIn={zoomIn}
        onZoomOut={zoomOut}
      />

      {notice && <NoticeBar notice={notice} onDismiss={() => setNotice(null)} />}

      <main
        className="relative flex-auto min-h-0 overflow-hidden"
        aria-label={stageActive ? "Schematic preview" : "Welcome"}
      >
        <WelcomePanel
          bootstrap={bootstrap}
          busy={busy}
          hidden={stageActive}
          onLoadDemo={loadDemo}
          onOpenFile={openFile}
        />

        <PreviewStage
          canvasRef={canvasRef}
          loaded={loaded}
          loading={loading}
          onCancel={home}
          stageActive={stageActive}
        />

        {dragging && <DropOverlay />}
      </main>

      <PreviewFooter
        bootstrap={bootstrap}
        choosing={choosing}
        loaded={loaded}
        loading={loading}
        previewMemory={previewMemory}
      />

      <AppDialog
        actionBusy={actionBusy}
        bootstrap={bootstrap}
        dialog={dialog}
        errorDetails={errorDetails}
        onNativeAction={nativeAction}
        onOpenChange={setDialogOpen}
        onSettingsChange={updatePreviewSettings}
        onThemeChange={onThemeChange}
        open={dialogOpen}
        settings={previewSettings}
        theme={theme}
      />
    </FluentProvider>
  )
}
