import {
  Button,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Spinner,
  SplitButton,
  ToggleButton,
  Tooltip,
} from "@fluentui/react-components"
import {
  Add20Regular,
  Airplane20Regular,
  ArrowExpand20Regular,
  Copy20Regular,
  FolderOpen20Regular,
  Grid20Regular,
  Home20Regular,
  Save20Regular,
  Subtract20Regular,
} from "@fluentui/react-icons"

import { fileName, type Loaded, type Loading, type PreviewImageAction } from "./app-model"

export interface PreviewToolbarProps {
  busy: boolean
  flyMode: boolean
  grid: boolean
  imageAction: PreviewImageAction | null
  loaded: Loaded | null
  loading: Loading | null
  onCopyImage: () => void
  onExportImage: () => void
  onFit: () => void
  onHome: () => void
  onOpenFile: () => void
  onToggleFly: () => void
  onToggleGrid: () => void
  onZoomIn: () => void
  onZoomOut: () => void
}

export function PreviewToolbar({
  busy,
  flyMode,
  grid,
  imageAction,
  loaded,
  loading,
  onCopyImage,
  onExportImage,
  onFit,
  onHome,
  onOpenFile,
  onToggleFly,
  onToggleGrid,
  onZoomIn,
  onZoomOut,
}: PreviewToolbarProps) {
  const imageDisabled = !loaded || loading !== null || busy || imageAction !== null
  const copying = imageAction === "copy"
  const exporting = imageAction === "export"

  return (
    <nav
      className="border-border bg-surface flex min-h-14 flex-none items-center gap-1.5 border-b px-3 py-2 sm:gap-2 sm:px-5 sm:py-2.5 [&>button]:h-9! [&>button]:shrink-0 [&>button:has(span.hidden)]:gap-2! [&>button:has(span.hidden)]:px-3! [&>button:not(:has(span.hidden))]:w-9! [&>button:not(:has(span.hidden))]:min-w-9!"
      aria-label="Preview commands"
    >
      <Button
        appearance="primary"
        icon={<FolderOpen20Regular />}
        disabled={busy}
        onClick={onOpenFile}
        title="Open schematic (Ctrl+O)"
      >
        Open<span className="ml-1 hidden text-xs font-normal opacity-75 sm:inline">Ctrl+O</span>
      </Button>
      <Tooltip content="Return home" relationship="label">
        <Button appearance="subtle" icon={<Home20Regular />} onClick={onHome} aria-label="Home" />
      </Tooltip>
      <span className="bg-border mx-0.5 h-5 w-px shrink-0 self-center sm:mx-1" />
      <Button
        appearance="subtle"
        icon={<ArrowExpand20Regular />}
        disabled={!loaded && !loading}
        onClick={onFit}
        title="Fit schematic (F)"
      >
        Fit<span className="ml-1 hidden text-xs font-normal opacity-75 sm:inline">F</span>
      </Button>
      <Tooltip content="Zoom out (−)" relationship="label">
        <Button
          appearance="subtle"
          icon={<Subtract20Regular />}
          disabled={(!loaded && !loading) || flyMode}
          onClick={onZoomOut}
          aria-label="Zoom out"
        />
      </Tooltip>
      <Tooltip content="Zoom in (+)" relationship="label">
        <Button
          appearance="subtle"
          icon={<Add20Regular />}
          disabled={(!loaded && !loading) || flyMode}
          onClick={onZoomIn}
          aria-label="Zoom in"
        />
      </Tooltip>
      <span className="bg-border mx-0.5 h-5 w-px shrink-0 self-center sm:mx-1" />
      <Tooltip content={grid ? "Hide ground grid" : "Show ground grid"} relationship="label">
        <ToggleButton
          appearance="subtle"
          icon={<Grid20Regular />}
          checked={grid}
          onClick={onToggleGrid}
          aria-label="Ground grid"
        />
      </Tooltip>
      <span className="bg-border mx-0.5 h-5 w-px shrink-0 self-center sm:mx-1" />
      <Tooltip content={flyMode ? "Return to orbit (V)" : "Spectator fly (V)"} relationship="label">
        <ToggleButton
          appearance="subtle"
          icon={<Airplane20Regular />}
          checked={flyMode}
          disabled={!loaded}
          onClick={onToggleFly}
          aria-label="Fly"
          aria-keyshortcuts="V"
        >
          Fly<span className="ml-1 hidden text-xs font-normal opacity-75 sm:inline">V</span>
        </ToggleButton>
      </Tooltip>
      <span className="bg-border mx-0.5 h-5 w-px shrink-0 self-center sm:mx-1" />
      <Menu>
        <MenuTrigger disableButtonEnhancement>
          {(triggerProps) => (
            <SplitButton
              appearance="subtle"
              aria-busy={imageAction !== null}
              className="h-9! shrink-0 [&>button]:h-9!"
              disabled={imageDisabled}
              icon={copying ? <Spinner aria-hidden="true" size="tiny" /> : <Copy20Regular />}
              menuButton={{
                ...triggerProps,
                "aria-label": "Image export options",
                title: "Save the preview as PNG or JPEG",
              }}
              primaryActionButton={{
                "aria-label": copying
                  ? "Copying preview image to clipboard"
                  : "Copy preview image to clipboard",
                onClick: onCopyImage,
                title: "Copy preview image to clipboard",
              }}
            >
              {copying ? "Copying…" : "Copy as Image"}
            </SplitButton>
          )}
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuItem
              disabled={imageDisabled}
              icon={exporting ? <Spinner aria-hidden="true" size="tiny" /> : <Save20Regular />}
              onClick={onExportImage}
            >
              {exporting ? "Saving image…" : "Save image…"}
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>
      <span
        className="text-muted ml-auto hidden max-w-[40%] overflow-hidden pl-4 text-xs text-ellipsis whitespace-nowrap sm:block md:max-w-[30%]"
        title={loaded?.path || loading?.path}
      >
        {loaded ? fileName(loaded.path) : loading ? fileName(loading.path) : null}
      </span>
    </nav>
  )
}
