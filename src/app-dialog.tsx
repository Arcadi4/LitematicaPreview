import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  ProgressBar,
  Radio,
  RadioGroup,
  Tab,
  TabList,
} from "@fluentui/react-components"
import { useId, useState } from "react"

import icon from "../Assets/app-ui.png"
import {
  appName,
  isTheme,
  type Bootstrap,
  type DialogKind,
  type NativeCommand,
  type TexturePackState,
  type ThemePreference,
  type UpdateStatus,
} from "./app-model"
import type { PreviewSettings } from "./preview-settings"
import { SettingsPanel } from "./settings-panel"
import { TexturePacksPanel } from "./texture-packs-panel"

export interface AppDialogProps {
  actionBusy: boolean
  bootstrap: Bootstrap | null
  dialog: DialogKind
  errorDetails: string
  onCheckForUpdate: () => void
  onDownloadUpdate: () => void
  onImportTexturePacks: () => void
  onInstallUpdate: () => void
  onNativeAction: (command: NativeCommand) => void
  onOpenChange: (open: boolean) => void
  onRegisterAssociations: () => void
  onRemoveTexturePack: (id: string) => void
  onSetTexturePackEnabled: (id: string, enabled: boolean) => void
  onSetTexturePackOrder: (ids: string[]) => void
  onSettingsChange: (patch: Partial<PreviewSettings>) => void
  onThemeChange: (theme: ThemePreference) => void
  open: boolean
  settings: PreviewSettings
  texturePackBusy: boolean
  texturePacks: TexturePackState
  theme: ThemePreference
  update: UpdateStatus
}

const title = (dialog: DialogKind) =>
  dialog === "error"
    ? "Unable to open preview"
    : dialog === "settings"
      ? "Settings"
      : dialog === "associations"
        ? "File associations"
        : `About ${appName}`

function ControlsHelp() {
  return (
    <>
      <p className="mt-0 mb-5 leading-relaxed">
        Click or Tab into the preview to use its keyboard controls.
      </p>
      <dl className="my-0 mb-5 flex flex-col gap-0 [&_dd]:m-0 [&_dd]:leading-normal [&_dt]:font-semibold [&>div]:grid [&>div]:grid-cols-[80px_1fr] [&>div]:gap-3 [&>div]:border-b [&>div]:border-[var(--colorNeutralStroke2,#dddddd)] [&>div]:py-3 sm:[&>div]:grid-cols-[120px_1fr] sm:[&>div]:gap-4">
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
          <dt>Fly</dt>
          <dd>
            <kbd>V</kbd> toggles spectator fly: WASD, Space / Ctrl, mouse look
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

function SettingsSections({
  bootstrap,
  onImportTexturePacks,
  onRemoveTexturePack,
  onSetTexturePackEnabled,
  onSetTexturePackOrder,
  onSettingsChange,
  onThemeChange,
  settings,
  texturePackBusy,
  texturePacks,
  theme,
}: Pick<
  AppDialogProps,
  | "bootstrap"
  | "onImportTexturePacks"
  | "onRemoveTexturePack"
  | "onSetTexturePackEnabled"
  | "onSetTexturePackOrder"
  | "onSettingsChange"
  | "onThemeChange"
  | "settings"
  | "texturePackBusy"
  | "texturePacks"
  | "theme"
>) {
  const [section, setSection] = useState("performance")
  const id = useId()
  return (
    <div className="settings-layout">
      <TabList
        aria-label="Settings sections"
        className="settings-sidebar"
        vertical
        selectedValue={section}
        onTabSelect={(_, data) => {
          if (typeof data.value === "string") setSection(data.value)
        }}
      >
        <Tab id={`${id}-performance-tab`} aria-controls={`${id}-performance`} value="performance">
          Performance
        </Tab>
        <Tab id={`${id}-textures-tab`} aria-controls={`${id}-textures`} value="textures">
          Texture packs
        </Tab>
        <Tab id={`${id}-control-tab`} aria-controls={`${id}-control`} value="control">
          Control
        </Tab>
        <Tab id={`${id}-appearance-tab`} aria-controls={`${id}-appearance`} value="appearance">
          Appearance
        </Tab>
      </TabList>
      <div key={section} className="settings-content-scroll">
        <div
          role="tabpanel"
          id={`${id}-performance`}
          aria-labelledby={`${id}-performance-tab`}
          hidden={section !== "performance"}
          tabIndex={0}
        >
          {section === "performance" && (
            <div>
              <h2 className="mt-0 mb-5 text-lg font-semibold">Performance</h2>
              <SettingsPanel
                bootstrap={bootstrap}
                onChange={onSettingsChange}
                settings={settings}
              />
            </div>
          )}
        </div>
        <div
          role="tabpanel"
          id={`${id}-textures`}
          aria-labelledby={`${id}-textures-tab`}
          hidden={section !== "textures"}
          tabIndex={0}
        >
          {section === "textures" && (
            <div>
              <h2 className="mt-0 mb-5 text-lg font-semibold">Texture packs</h2>
              <TexturePacksPanel
                busy={texturePackBusy}
                onImport={onImportTexturePacks}
                onRemove={onRemoveTexturePack}
                onSetEnabled={onSetTexturePackEnabled}
                onSetOrder={onSetTexturePackOrder}
                state={texturePacks}
              />
            </div>
          )}
        </div>
        <div
          role="tabpanel"
          id={`${id}-control`}
          aria-labelledby={`${id}-control-tab`}
          hidden={section !== "control"}
          tabIndex={0}
        >
          {section === "control" && (
            <div>
              <h2 className="mt-0 mb-5 text-lg font-semibold">Control</h2>
              <ControlsHelp />
            </div>
          )}
        </div>
        <div
          role="tabpanel"
          id={`${id}-appearance`}
          aria-labelledby={`${id}-appearance-tab`}
          hidden={section !== "appearance"}
          tabIndex={0}
        >
          {section === "appearance" && (
            <div className="flex flex-col gap-4">
              <h2 className="m-0 mb-1 text-lg font-semibold">Appearance</h2>
              <Field label="Theme">
                <RadioGroup
                  value={theme}
                  onChange={(_, data) => {
                    if (isTheme(data.value)) onThemeChange(data.value)
                  }}
                >
                  <Radio value="system" label="Use system setting" />
                  <Radio value="light" label="Light" />
                  <Radio value="dark" label="Dark" />
                </RadioGroup>
              </Field>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

function updateMessage(update: UpdateStatus): string {
  if (update.phase === "checking") return "Checking for updates…"
  if (update.phase === "available") return `Version ${update.version} is available.`
  if (update.phase === "downloading")
    return `Downloading version ${update.version}… ${update.percent}%`
  if (update.phase === "ready") return `Version ${update.version} is ready to install.`
  if (update.phase === "applying") return "Restarting to install the update…"
  if (update.phase === "error") return "The update could not be completed."
  if (update.phase === "current") return "Litematica Preview is up to date."
  return "Updates are checked automatically."
}

function UpdateSection({
  bootstrap,
  update,
  onCheckForUpdate,
  onDownloadUpdate,
  onInstallUpdate,
}: Pick<
  AppDialogProps,
  "bootstrap" | "update" | "onCheckForUpdate" | "onDownloadUpdate" | "onInstallUpdate"
>) {
  if (!bootstrap?.updatesSupported) return null
  const busy =
    update.phase === "checking" || update.phase === "downloading" || update.phase === "applying"
  const showNotes =
    (update.phase === "available" || update.phase === "downloading" || update.phase === "ready") &&
    update.notes.trim().length > 0
  return (
    <div className="mt-6 flex flex-col gap-2">
      <strong className="text-sm">Updates</strong>
      <p className="m-0 leading-relaxed">{updateMessage(update)}</p>
      {update.phase === "downloading" && (
        <ProgressBar value={update.percent / 100} aria-label="Update download progress" />
      )}
      {update.phase === "error" && update.message && (
        <p className="m-0 leading-relaxed [overflow-wrap:anywhere]">{update.message}</p>
      )}
      {showNotes && (
        <details>
          <summary className="cursor-pointer select-none">Release notes</summary>
          <div className="mt-2 max-h-48 overflow-auto text-sm leading-relaxed whitespace-pre-wrap">
            {update.notes}
          </div>
        </details>
      )}
      <div className="flex gap-2">
        {update.phase === "available" ? (
          <Button appearance="primary" onClick={onDownloadUpdate}>
            Download update
          </Button>
        ) : update.phase === "ready" ? (
          <Button appearance="primary" onClick={onInstallUpdate}>
            Restart and update
          </Button>
        ) : (
          <Button disabled={busy} onClick={onCheckForUpdate}>
            Check for updates
          </Button>
        )}
      </div>
    </div>
  )
}

function AssociationOptions({ bootstrap }: Pick<AppDialogProps, "bootstrap">) {
  return (
    <div className="leading-relaxed [&>p]:mb-3">
      <p>
        Windows keeps your current default apps. Litematica Preview can appear in Open with for
        schematic files without changing those defaults.
      </p>
      <p>
        Register Litematica Preview for{" "}
        {bootstrap?.extensions.join(", ") ?? "its supported formats"}. You can change defaults later
        in Windows Settings.
      </p>
      {bootstrap?.legacyInstall && (
        <p>
          An older installation exists at <code>{bootstrap.legacyInstall}</code>. Uninstall it
          before registering this copy, and do not select its option to delete app data.
        </p>
      )}
    </div>
  )
}

function AboutLicenses({
  bootstrap,
  update,
  onCheckForUpdate,
  onDownloadUpdate,
  onInstallUpdate,
}: Pick<
  AppDialogProps,
  "bootstrap" | "update" | "onCheckForUpdate" | "onDownloadUpdate" | "onInstallUpdate"
>) {
  return (
    <div className="leading-relaxed [&>p]:mb-3">
      <div className="my-3 mb-6 flex items-center gap-3.5 [&_span]:text-xs [&_strong]:text-lg [&_strong]:font-semibold [&>div]:flex [&>div]:flex-col [&>div]:gap-1">
        <img className="flex-none object-contain" src={icon} width="48" height="48" alt="" />
        <div>
          <strong>{appName}</strong>
          <span>Version {bootstrap?.version || "unavailable"}</span>
        </div>
      </div>
      <p>
        A local, interactive viewer for Minecraft schematics and structures, inspired by
        LitematicaQL.
      </p>
      <p>Powered by Nucleation, Tauri, wgpu, React, and Fluent UI.</p>
      <UpdateSection
        bootstrap={bootstrap}
        update={update}
        onCheckForUpdate={onCheckForUpdate}
        onDownloadUpdate={onDownloadUpdate}
        onInstallUpdate={onInstallUpdate}
      />
      <p>
        Distributed under the GNU Affero General Public License v3. This program comes with no
        warranty. Redistribution is permitted under the terms of the bundled license.
      </p>
    </div>
  )
}

export function AppDialog({
  actionBusy,
  bootstrap,
  dialog,
  errorDetails,
  onCheckForUpdate,
  onDownloadUpdate,
  onImportTexturePacks,
  onInstallUpdate,
  onNativeAction,
  onOpenChange,
  onRegisterAssociations,
  onRemoveTexturePack,
  onSetTexturePackEnabled,
  onSetTexturePackOrder,
  onSettingsChange,
  onThemeChange,
  open,
  settings,
  texturePackBusy,
  texturePacks,
  theme,
  update,
}: AppDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(_, data) => onOpenChange(data.open)}>
      <DialogSurface className={dialog === "settings" ? "settings-dialog" : undefined}>
        <DialogBody>
          <DialogTitle>{title(dialog)}</DialogTitle>
          <DialogContent>
            {dialog === "error" ? (
              <>
                <p>The preview was closed. You are back on the home screen.</p>
                <pre className="max-h-[45vh] overflow-auto text-xs [overflow-wrap:anywhere] whitespace-pre-wrap select-text">
                  {errorDetails}
                </pre>
              </>
            ) : dialog === "settings" ? (
              open && (
                <SettingsSections
                  bootstrap={bootstrap}
                  onImportTexturePacks={onImportTexturePacks}
                  onRemoveTexturePack={onRemoveTexturePack}
                  onSetTexturePackEnabled={onSetTexturePackEnabled}
                  onSetTexturePackOrder={onSetTexturePackOrder}
                  onSettingsChange={onSettingsChange}
                  onThemeChange={onThemeChange}
                  settings={settings}
                  texturePackBusy={texturePackBusy}
                  texturePacks={texturePacks}
                  theme={theme}
                />
              )
            ) : dialog === "associations" ? (
              <AssociationOptions bootstrap={bootstrap} />
            ) : (
              <AboutLicenses
                bootstrap={bootstrap}
                update={update}
                onCheckForUpdate={onCheckForUpdate}
                onDownloadUpdate={onDownloadUpdate}
                onInstallUpdate={onInstallUpdate}
              />
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
            {dialog === "associations" ? (
              <>
                <Button onClick={() => onOpenChange(false)}>Not now</Button>
                <Button appearance="primary" onClick={onRegisterAssociations}>
                  Register file types
                </Button>
              </>
            ) : (
              <Button appearance="primary" onClick={() => onOpenChange(false)}>
                Close
              </Button>
            )}
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  )
}
