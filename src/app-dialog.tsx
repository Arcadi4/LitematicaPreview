import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
} from "@fluentui/react-components"
import icon from "../Assets/app-ui.png"
import { appName, type Bootstrap, type DialogKind, type NativeCommand } from "./app-model"
import type { PreviewSettings } from "./preview-settings"
import { SettingsPanel } from "./settings-panel"

export interface AppDialogProps {
  actionBusy: boolean
  bootstrap: Bootstrap | null
  dialog: DialogKind
  errorDetails: string
  onNativeAction: (command: NativeCommand) => void
  onOpenChange: (open: boolean) => void
  onSettingsChange: (patch: Partial<PreviewSettings>) => void
  open: boolean
  settings: PreviewSettings
}

const title = (dialog: DialogKind) =>
  dialog === "error"
    ? "Unable to open preview"
    : dialog === "controls"
      ? "Controls and shortcuts"
      : dialog === "settings"
        ? "Preview settings"
        : `About ${appName}`

function ControlsHelp() {
  return (
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
        Use the grid button to show or hide the ground grid. Home on the command bar returns to your
        bundled examples.
      </p>
    </>
  )
}

function AboutLicenses({ version }: { version: string | undefined }) {
  return (
    <div className="leading-relaxed [&>p]:mb-3">
      <div className="flex items-center gap-3.5 my-3 mb-6 [&>div]:flex [&>div]:flex-col [&>div]:gap-1 [&_strong]:text-lg [&_strong]:font-semibold [&_span]:text-xs">
        <img className="object-contain flex-none" src={icon} width="48" height="48" alt="" />
        <div>
          <strong>{appName}</strong>
          <span>Version {version || "unavailable"}</span>
        </div>
      </div>
      <p>
        A local, interactive viewer for Minecraft schematics and structures, inspired by
        LitematicaQL.
      </p>
      <p>Powered by Nucleation, Tauri, WebGL, React, and Fluent UI.</p>
      <p>
        Distributed under the GNU Affero General Public License v3. This program comes with no
        warranty. Redistribution is permitted under the terms of the bundled license.
      </p>
      <p className="text-muted">
        The application license and third-party notices are included with your installation.
      </p>
    </div>
  )
}

export function AppDialog({
  actionBusy,
  bootstrap,
  dialog,
  errorDetails,
  onNativeAction,
  onOpenChange,
  onSettingsChange,
  open,
  settings,
}: AppDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface>
        <DialogBody>
          <DialogTitle>{title(dialog)}</DialogTitle>
          <DialogContent>
            {dialog === "error" ? (
              <>
                <p>The preview was closed. You are back on the home screen.</p>
                <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] max-h-[45vh] overflow-auto select-text text-xs">
                  {errorDetails}
                </pre>
              </>
            ) : dialog === "settings" ? (
              <SettingsPanel
                bootstrap={bootstrap}
                onChange={onSettingsChange}
                settings={settings}
              />
            ) : dialog === "controls" ? (
              <ControlsHelp />
            ) : (
              <AboutLicenses version={bootstrap?.version} />
            )}
          </DialogContent>
          <DialogActions>
            {dialog === "about" && (
              <Button
                disabled={actionBusy}
                onClick={() => {
                  onOpenChange(false)
                  onNativeAction("show_licenses")
                }}
              >
                Open licenses folder
              </Button>
            )}
            <Button appearance="primary" onClick={() => onOpenChange(false)}>
              Close
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  )
}
