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
    choosing,
    dialog,
    dialogOpen,
    dragging,
    errorDetails,
    fit,
    grid,
    home,
    importTexturePacks,
    loadDemo,
    loaded,
    loading,
    nativeAction,
    notice,
    openDialog,
    openFile,
    previewMemory,
    previewSettings,
    removeTexturePack,
    setDialogOpen,
    setNotice,
    setTexturePackEnabled,
    setTexturePackOrder,
    texturePackBusy,
    texturePacks,
    toggleGrid,
    updatePreviewSettings,
    viewportInput,
    viewportRef,
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
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark)
    document.documentElement.style.colorScheme = dark ? "dark" : "light"
    try {
      localStorage.setItem("litematica-preview-theme", theme)
    } catch {
      // Storage is optional; the theme still applies for this session.
    }
  }, [dark, theme])

  useEffect(() => {
    try {
      localStorage.setItem("litematica-preview-settings", JSON.stringify(previewSettings))
    } catch {
      // Storage is optional; the settings still apply for this session.
    }
  }, [previewSettings])

  // Effect Events keep the subscription stable while using current state.
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
      } else if (
        event.key.toLowerCase() === "f" &&
        (loaded || loading) &&
        target !== viewportRef.current
      ) {
        event.preventDefault()
        fit()
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [dialogOpen, fit, loaded, loading, viewportRef])

  const onThemeChange = (value: unknown) => {
    if (isTheme(value)) setTheme(value)
  }
  const stageActive = Boolean(loaded || loading)

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      // Portals inherit theme tokens, not the full-window app-shell layout.
      applyStylesToPortals={false}
      // Keep FluentProvider's opaque background off the native swapchain.
      className={`flex flex-col w-full h-full min-w-[320px] bg-transparent! text-text ${dark ? "dark theme-dark" : "theme-light"}`}
    >
      <AppHeader
        actionBusy={actionBusy}
        fileAssociationsSupported={bootstrap?.fileAssociationsSupported ?? false}
        onNativeAction={nativeAction}
        onOpenDialog={openDialog}
      />

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
          loaded={loaded}
          loading={loading}
          onCancel={home}
          onViewportInput={viewportInput}
          stageActive={stageActive}
          viewportRef={viewportRef}
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
        onImportTexturePacks={importTexturePacks}
        onNativeAction={nativeAction}
        onOpenChange={setDialogOpen}
        onRemoveTexturePack={removeTexturePack}
        onSetTexturePackEnabled={setTexturePackEnabled}
        onSetTexturePackOrder={setTexturePackOrder}
        onSettingsChange={updatePreviewSettings}
        onThemeChange={onThemeChange}
        open={dialogOpen}
        settings={previewSettings}
        texturePackBusy={texturePackBusy}
        texturePacks={texturePacks}
        theme={theme}
      />
    </FluentProvider>
  )
}
