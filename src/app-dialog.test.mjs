// @vitest-environment jsdom
import assert from "node:assert/strict"
import { act, createElement } from "react"
import { createRoot } from "react-dom/client"
import { FluentProvider, webLightTheme } from "@fluentui/react-components"
import { getByRole } from "@testing-library/dom"
import { afterAll, afterEach, beforeAll, test } from "vite-plus/test"
import { AppDialog } from "./app-dialog.tsx"
import { AppHeader } from "./app-header.tsx"
import { defaultPreviewSettings } from "./preview-settings.ts"

const previousActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT
const roots = new Set()
const bootstrap = {
  extensions: [],
  demos: [],
  initialPath: null,
  version: "0.3.0",
  requestId: 0,
  maxWorkerThreads: 8,
}
const dialogProps = {
  actionBusy: false,
  bootstrap,
  dialog: "settings",
  errorDetails: "",
  onNativeAction: () => {},
  onOpenChange: () => {},
  onSettingsChange: () => {},
  onThemeChange: () => {},
  open: true,
  settings: defaultPreviewSettings,
  theme: "system",
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
})

afterEach(async () => {
  await act(async () => {
    for (const root of roots) root.unmount()
  })
  roots.clear()
  document.body.replaceChildren()
})

afterAll(() => {
  if (previousActEnvironment === undefined) delete globalThis.IS_REACT_ACT_ENVIRONMENT
  else globalThis.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment
})

async function mount(Component, props) {
  const container = document.createElement("div")
  document.body.append(container)
  const root = createRoot(container)
  roots.add(root)
  const update = async (next) => {
    await act(async () => {
      root.render(
        createElement(FluentProvider, { theme: webLightTheme }, createElement(Component, next)),
      )
    })
  }
  await update(props)
  return update
}

function byRole(role, text) {
  return getByRole(document.body, role, { name: text })
}

async function click(element) {
  await act(async () => element.click())
}

function selectedPanel() {
  const tab = document.querySelector('[role="tab"][aria-selected="true"]')
  assert.ok(tab)
  const panel = document.getElementById(tab.getAttribute("aria-controls"))
  assert.ok(panel)
  assert.equal(panel.getAttribute("role"), "tabpanel")
  assert.equal(panel.getAttribute("aria-labelledby"), tab.id)
  assert.equal(panel.hidden, false)
  return panel
}

test("Settings defaults to Performance and switches to the existing controls reference", async () => {
  await mount(AppDialog, dialogProps)
  assert.equal(byRole("tablist", "Settings sections").getAttribute("aria-orientation"), "vertical")
  for (const name of ["Performance", "Control", "Appearance"]) byRole("tab", name)
  assert.equal(byRole("tab", "Performance").getAttribute("aria-selected"), "true")
  assert.ok(selectedPanel().querySelector('input[aria-label="Worker threads"]'))
  assert.equal(getByRole(selectedPanel(), "heading", { name: "Performance" }).tagName, "H2")

  await click(byRole("tab", "Control"))
  const control = selectedPanel()
  for (const label of ["Orbit", "Pan", "Zoom", "Fit model", "Open schematic", "Cancel loading"]) {
    assert.ok([...control.querySelectorAll("dt")].some((node) => node.textContent === label))
  }
  assert.equal(getByRole(control, "heading", { name: "Control" }).tagName, "H2")
  assert.equal(document.querySelector('input[aria-label="Worker threads"]'), null)
})

test("Appearance selects the existing theme immediately without changing performance options", async () => {
  const themes = []
  const settings = []
  const update = await mount(AppDialog, {
    ...dialogProps,
    onThemeChange: (value) => themes.push(value),
    onSettingsChange: (value) => settings.push(value),
  })
  await click(byRole("tab", "Appearance"))
  assert.equal(getByRole(selectedPanel(), "heading", { name: "Appearance" }).tagName, "H2")
  const system = selectedPanel().querySelector('input[value="system"]')
  assert.ok(system.checked)
  await click(selectedPanel().querySelector('input[value="dark"]'))
  assert.deepEqual(themes, ["dark"])
  assert.deepEqual(settings, [])

  await update({ ...dialogProps, theme: "dark" })
  assert.ok(selectedPanel().querySelector('input[value="dark"]').checked)
  await click(byRole("tab", "Performance"))
  assert.ok(selectedPanel().querySelector('input[aria-label="Worker threads"]'))
})

test("closing and reopening Settings resets navigation to Performance", async () => {
  const update = await mount(AppDialog, dialogProps)
  await click(byRole("tab", "Appearance"))
  await update({ ...dialogProps, open: false })
  await update(dialogProps)
  assert.equal(byRole("tab", "Performance").getAttribute("aria-selected"), "true")
})

test("switching settings sections resets content scroll without moving navigation or Close", async () => {
  await mount(AppDialog, dialogProps)
  await click(byRole("tab", "Control"))
  const navigation = byRole("tablist", "Settings sections")
  const close = byRole("button", "Close")
  const content = selectedPanel().parentElement
  content.scrollTop = 180
  assert.equal(content.contains(navigation), false)
  assert.equal(content.contains(close), false)
  await click(byRole("tab", "Appearance"))
  assert.equal(selectedPanel().parentElement.scrollTop, 0)
  assert.equal(byRole("tablist", "Settings sections"), navigation)
  assert.equal(byRole("button", "Close"), close)
  await click(byRole("tab", "Control"))
  assert.equal(selectedPanel().parentElement.scrollTop, 0)
})

test("the application menu has one Settings entry and retains About and file associations", async () => {
  const dialogs = []
  await mount(AppHeader, {
    actionBusy: false,
    onNativeAction: () => {},
    onOpenDialog: (dialog) => dialogs.push(dialog),
  })
  await click(document.querySelector('button[aria-label="Application menu"]'))
  const settings = byRole("menuitem", "Settings")
  byRole("menuitem", "About and licenses")
  byRole("menuitem", "Set as default app…")
  byRole("menuitem", "Remove file associations")
  await click(settings)
  assert.deepEqual(dialogs, ["settings"])
})
