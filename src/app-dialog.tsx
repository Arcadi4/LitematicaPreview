import {
  Button,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
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
  type ThemePreference,
} from "./app-model"
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
  onThemeChange: (theme: ThemePreference) => void
  open: boolean
  settings: PreviewSettings
  theme: ThemePreference
}

const title = (dialog: DialogKind) =>
  dialog === "error"
    ? "Unable to open preview"
    : dialog === "settings"
      ? "Settings"
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

function SettingsSections({
  bootstrap,
  onSettingsChange,
  onThemeChange,
  settings,
  theme,
}: Pick<
  AppDialogProps,
  "bootstrap" | "onSettingsChange" | "onThemeChange" | "settings" | "theme"
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
              <p className="m-0 leading-relaxed">
                Theme changes apply immediately and are saved automatically.
              </p>
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
  onThemeChange,
  open,
  settings,
  theme,
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
                <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] max-h-[45vh] overflow-auto select-text text-xs">
                  {errorDetails}
                </pre>
              </>
            ) : dialog === "settings" ? (
              open && (
                <SettingsSections
                  bootstrap={bootstrap}
                  onSettingsChange={onSettingsChange}
                  onThemeChange={onThemeChange}
                  settings={settings}
                  theme={theme}
                />
              )
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
