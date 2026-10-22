// @vitest-environment jsdom
import assert from "node:assert/strict"
import { act, createElement, type ComponentType } from "react"
import { createRoot, type Root } from "react-dom/client"
import { FluentProvider, webLightTheme } from "@fluentui/react-components"
import { getByRole, queryByRole, type ByRoleMatcher } from "@testing-library/dom"
import { afterAll, afterEach, beforeAll, test } from "vite-plus/test"
import { AppDialog, type AppDialogProps } from "./app-dialog"
import { AppHeader } from "./app-header"
import type { Bootstrap, ThemePreference } from "./app-model"
import { defaultPreviewSettings, type PreviewSettings } from "./preview-settings"

declare global {
  // React reads this flag; its type declaration omits it.
  // eslint-disable-next-line no-var
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined
}

const previousActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT
const roots = new Set<Root>()
const bootstrap: Bootstrap = {
  extensions: [],
  demos: [],
  initialPath: null,
  version: "0.3.0",
  requestId: 0,
  maxWorkerThreads: 8,
  fileAssociationsSupported: true,
  viewportError: null,
  texturePacks: { packs: [] },
  texturePackError: null,
}
const dialogProps: AppDialogProps = {
  actionBusy: false,
  bootstrap,
  dialog: "settings",
  errorDetails: "",
  onImportTexturePacks: () => {},
  onNativeAction: () => {},
  onOpenChange: () => {},
  onRemoveTexturePack: () => {},
  onSetTexturePackEnabled: () => {},
  onSetTexturePackOrder: () => {},
  onSettingsChange: () => {},
  onThemeChange: () => {},
  open: true,
  settings: defaultPreviewSettings,
  texturePackBusy: false,
  texturePacks: { packs: [] },
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

async function mount<Props extends object>(Component: ComponentType<Props>, props: Props) {
  const container = document.createElement("div")
  document.body.append(container)
  const root = createRoot(container)
  roots.add(root)
  const update = async (next: Props) => {
    await act(async () => {
      root.render(
        createElement(FluentProvider, { theme: webLightTheme }, createElement(Component, next)),
      )
    })
  }
  await update(props)
  return update
}

function byRole(role: ByRoleMatcher, text: string) {
  return getByRole(document.body, role, { name: text })
}

async function click(element: HTMLElement | null) {
  assert.ok(element, "Expected an element to click")
  const target = element
  await act(async () => target.click())
}

function selectedPanel() {
  const tab = document.querySelector('[role="tab"][aria-selected="true"]')
  assert.ok(tab, "Settings must select a tab")
  const controls = tab.getAttribute("aria-controls")
  assert.ok(controls, "The selected tab must point at its panel")
  const panel = document.getElementById(controls)
  assert.ok(panel, `Missing panel for ${controls}`)
  assert.equal(panel.getAttribute("role"), "tabpanel")
  assert.equal(panel.getAttribute("aria-labelledby"), tab.id)
  assert.equal(panel.hidden, false)
  return panel
}

/** Finds a theme radio and fails clearly if it is absent. */
function radio(value: string) {
  const element = selectedPanel().querySelector<HTMLInputElement>(`input[value="${value}"]`)
  assert.ok(element, `Missing radio input for ${value}`)
  return element
}

test("Settings defaults to Performance and switches to the existing controls reference", async () => {
  await mount(AppDialog, dialogProps)
  assert.equal(byRole("tablist", "Settings sections").getAttribute("aria-orientation"), "vertical")
  for (const name of ["Performance", "Texture packs", "Control", "Appearance"]) byRole("tab", name)
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
  const themes: ThemePreference[] = []
  const settings: Partial<PreviewSettings>[] = []
  const update = await mount(AppDialog, {
    ...dialogProps,
    onThemeChange: (value) => {
      themes.push(value)
    },
    onSettingsChange: (value) => {
      settings.push(value)
    },
  })
  await click(byRole("tab", "Appearance"))
  assert.equal(getByRole(selectedPanel(), "heading", { name: "Appearance" }).tagName, "H2")
  assert.ok(radio("system").checked)
  await click(radio("dark"))
  assert.deepEqual(themes, ["dark"])
  assert.deepEqual(settings, [])

  await update({ ...dialogProps, theme: "dark" })
  assert.ok(radio("dark").checked)
  await click(byRole("tab", "Performance"))
  assert.ok(selectedPanel().querySelector('input[aria-label="Worker threads"]'))
})

test("Texture packs toggle, reorder highest first, and confirm per-row removal", async () => {
  const orders: string[][] = []
  const toggles: Array<[string, boolean]> = []
  const removed: string[] = []
  await mount(AppDialog, {
    ...dialogProps,
    onRemoveTexturePack: (id) => {
      removed.push(id)
    },
    onSetTexturePackEnabled: (id, enabled) => {
      toggles.push([id, enabled])
    },
    onSetTexturePackOrder: (ids) => {
      orders.push(ids)
    },
    texturePacks: {
      packs: [
        { id: "alpha", name: "Alpha", enabled: true },
        { id: "beta", name: "Beta", enabled: false },
      ],
    },
  })
  await click(byRole("tab", "Texture packs"))
  const panel = selectedPanel()
  assert.equal(getByRole(panel, "heading", { name: "Texture packs" }).tagName, "H2")
  assert.match(panel.textContent ?? "", /Bundled vanilla resources/)
  const rows = [...panel.querySelectorAll("ol > li")]
  assert.match(rows[0]?.textContent ?? "", /Alpha/)
  assert.match(rows[1]?.textContent ?? "", /Beta/)

  const alphaUp = getByRole(panel, "button", { name: "Move Alpha up" }) as HTMLButtonElement
  const betaDown = getByRole(panel, "button", { name: "Move Beta down" }) as HTMLButtonElement
  assert.equal(alphaUp.disabled, true, "the highest pack cannot move up")
  assert.equal(betaDown.disabled, true, "the lowest pack cannot move down")
  await click(getByRole(panel, "button", { name: "Move Beta up" }))
  assert.deepEqual(orders, [["beta", "alpha"]])

  assert.equal((getByRole(panel, "switch", { name: "Alpha" }) as HTMLInputElement).checked, true)
  assert.equal((getByRole(panel, "switch", { name: "Beta" }) as HTMLInputElement).checked, false)
  await click(getByRole(panel, "switch", { name: "Beta" }))
  assert.deepEqual(toggles, [["beta", true]])

  await click(getByRole(panel, "button", { name: "Remove Alpha from saved packs" }))
  assert.deepEqual(removed, [])
  await click(getByRole(selectedPanel(), "button", { name: "Remove “Alpha”" }))
  assert.deepEqual(removed, ["alpha"])
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
  assert.ok(content, "The selected panel must have a scroll container")
  content.scrollTop = 180
  assert.equal(content.contains(navigation), false)
  assert.equal(content.contains(close), false)
  await click(byRole("tab", "Appearance"))
  assert.equal(selectedPanel().parentElement?.scrollTop, 0)
  assert.equal(byRole("tablist", "Settings sections"), navigation)
  assert.equal(byRole("button", "Close"), close)
  await click(byRole("tab", "Control"))
  assert.equal(selectedPanel().parentElement?.scrollTop, 0)
})

test("association actions follow host support and busy state without hiding other commands", async () => {
  const props = {
    actionBusy: false,
    fileAssociationsSupported: false,
    onNativeAction: () => {},
    onOpenDialog: () => {},
  }
  const update = await mount(AppHeader, props)
  await click(byRole("button", "Application menu"))
  assert.equal(queryByRole(document.body, "menuitem", { name: "Set as default app…" }), null)
  assert.equal(queryByRole(document.body, "menuitem", { name: "Remove file associations" }), null)
  byRole("menuitem", "Settings")
  byRole("menuitem", "About and licenses")

  await update({ ...props, fileAssociationsSupported: true })
  assert.notEqual(byRole("menuitem", "Set as default app…").getAttribute("aria-disabled"), "true")
  assert.notEqual(
    byRole("menuitem", "Remove file associations").getAttribute("aria-disabled"),
    "true",
  )

  await update({ ...props, fileAssociationsSupported: true, actionBusy: true })
  assert.equal(byRole("menuitem", "Set as default app…").getAttribute("aria-disabled"), "true")
  assert.equal(byRole("menuitem", "Remove file associations").getAttribute("aria-disabled"), "true")
})
