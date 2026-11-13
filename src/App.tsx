import { FluentProvider, webDarkTheme, webLightTheme } from "@fluentui/react-components"
import { useEffect, useEffectEvent, useRef, useState } from "react"

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

const ASSOCIATION_PROMPT_KEY = "litematica-preview-association-prompt"

export default function App({ initialError }: { initialError?: string }) {
  const [theme, setTheme] = useState<ThemePreference>(savedTheme)
  const [systemDark, setSystemDark] = useState(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  )

  const {
    actionBusy,
    bootstrap,
    busy,
    checkForUpdate,
    copyImage,
    dialog,
    dialogOpen,
    downloadUpdate,
    dragging,
    errorDetails,
    exitFly,
    exportImage,
    fit,
    flyMode,
    grid,
    home,
    imageAction,
    importTexturePacks,
    installUpdate,
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
    toggleFly,
    toggleGrid,
    update,
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
      if (event.key === "Escape" && flyMode) {
        event.preventDefault()
        exitFly()
      } else if (event.key === "Escape" && loading) {
        event.preventDefault()
        returnHome()
      } else if (event.key.toLowerCase() === "v" && loaded && !event.repeat) {
        event.preventDefault()
        toggleFly()
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
  }, [dialogOpen, exitFly, fit, flyMode, loaded, loading, toggleFly, viewportRef])

  const onThemeChange = (value: unknown) => {
    if (isTheme(value)) setTheme(value)
  }
  const associationPromptShown = useRef(false)
  useEffect(() => {
    if (
      !bootstrap ||
      !bootstrap.updatesSupported ||
      !bootstrap.fileAssociationsSupported ||
      bootstrap.fileAssociationsRegistered ||
      associationPromptShown.current
    )
      return
    associationPromptShown.current = true
    try {
      if (localStorage.getItem(ASSOCIATION_PROMPT_KEY) === "1") return
    } catch {
      // Storage is optional; the prompt still appears for this session.
    }
    openDialog("associations")
  }, [bootstrap, openDialog])

  const closeDialog = (open: boolean) => {
    setDialogOpen(open)
    if (!open && dialog === "associations") {
      try {
        localStorage.setItem(ASSOCIATION_PROMPT_KEY, "1")
      } catch {
        // Storage is optional; the prompt returns on the next launch.
      }
    }
  }

  const registerAssociations = () => {
    nativeAction("register_associations")
    closeDialog(false)
  }
  const stageActive = Boolean(loaded || loading)

  return (
    <FluentProvider
      theme={dark ? webDarkTheme : webLightTheme}
      // Portals inherit theme tokens, not the full-window app-shell layout.
      applyStylesToPortals={false}
      // Keep FluentProvider's opaque background off the native swapchain.
      className={`text-text flex h-full w-full min-w-[320px] flex-col bg-transparent! ${dark ? "dark theme-dark" : "theme-light"}`}
    >
      <AppHeader
        actionBusy={actionBusy}
        fileAssociationsSupported={bootstrap?.fileAssociationsSupported ?? false}
        onNativeAction={nativeAction}
        onOpenDialog={openDialog}
      />

      <PreviewToolbar
        busy={busy}
        flyMode={flyMode}
        grid={grid}
        imageAction={imageAction}
        loaded={loaded}
        loading={loading}
        onCopyImage={copyImage}
        onExportImage={exportImage}
        onFit={fit}
        onHome={home}
        onOpenFile={openFile}
        onToggleFly={toggleFly}
        onToggleGrid={toggleGrid}
        onZoomIn={zoomIn}
        onZoomOut={zoomOut}
      />

      <NoticeBar
        notice={stageActive ? null : notice}
        onDismiss={() => setNotice(null)}
        className="flex-none"
      />

      <main
        className="relative min-h-0 flex-auto overflow-hidden"
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
          flyMode={flyMode}
          loaded={loaded}
          loading={loading}
          onCancel={home}
          onExitFly={exitFly}
          onViewportInput={viewportInput}
          stageActive={stageActive}
          viewportRef={viewportRef}
        />

        {/* Floating keeps the stage size, and with it the camera projection, unchanged. */}
        <NoticeBar
          notice={stageActive ? notice : null}
          onDismiss={() => setNotice(null)}
          className="absolute inset-x-0 top-0 z-10"
        />

        {dragging && <DropOverlay />}
      </main>

      <PreviewFooter
        bootstrap={bootstrap}
        loaded={loaded}
        loading={loading}
        previewMemory={previewMemory}
      />

      <AppDialog
        actionBusy={actionBusy}
        bootstrap={bootstrap}
        dialog={dialog}
        errorDetails={errorDetails}
        onCheckForUpdate={checkForUpdate}
        onDownloadUpdate={downloadUpdate}
        onImportTexturePacks={importTexturePacks}
        onInstallUpdate={installUpdate}
        onNativeAction={nativeAction}
        onOpenChange={closeDialog}
        onRegisterAssociations={registerAssociations}
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
        update={update}
      />
    </FluentProvider>
  )
}
