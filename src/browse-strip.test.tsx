// @vitest-environment jsdom
import assert from "node:assert/strict"

import { FluentProvider, webLightTheme } from "@fluentui/react-components"
import type { InvokeArgs } from "@tauri-apps/api/core"
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event"
import { act, createElement } from "react"
import { createRoot, type Root } from "react-dom/client"
import { afterAll, afterEach, beforeAll, beforeEach, test, vi } from "vite-plus/test"

import { BrowseStrip, type BrowseStripProps } from "./browse-strip"

const tauri = vi.hoisted(() => ({
  invoke: vi.fn<(command: string, args?: InvokeArgs) => Promise<unknown>>(),
  listen: vi.fn<(event: string, handler: EventCallback<unknown>) => Promise<UnlistenFn>>(),
}))

vi.mock("@tauri-apps/api/core", () => ({ invoke: tauri.invoke }))
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }))

declare global {
  // React reads this flag; its type declaration omits it.
  // eslint-disable-next-line no-var
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined
}

const previousActEnvironment = globalThis.IS_REACT_ACT_ENVIRONMENT
const roots = new Set<Root>()

class ResizeObserverStub {
  observe() {
    // jsdom has no layout, so the strip keeps its zero viewport.
  }
  unobserve() {
    // No observed targets are tracked.
  }
  disconnect() {
    // Nothing to release.
  }
}

beforeAll(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
  vi.stubGlobal("ResizeObserver", ResizeObserverStub)
})

beforeEach(() => {
  tauri.invoke.mockReset()
  // Keep thumbnails pending unless a test resolves them, so mounting is quiet.
  tauri.invoke.mockImplementation(() => Promise.withResolvers<never>().promise)
  tauri.listen.mockReset()
  tauri.listen.mockImplementation(async () => () => {})
})

afterEach(async () => {
  await act(async () => {
    for (const root of roots) root.unmount()
  })
  roots.clear()
  document.body.replaceChildren()
})

afterAll(() => {
  vi.unstubAllGlobals()
  if (previousActEnvironment === undefined) delete globalThis.IS_REACT_ACT_ENVIRONMENT
  else globalThis.IS_REACT_ACT_ENVIRONMENT = previousActEnvironment
})

async function mount(props: BrowseStripProps) {
  const container = document.createElement("div")
  document.body.append(container)
  const root = createRoot(container)
  roots.add(root)
  const update = async (next: BrowseStripProps) => {
    await act(async () => {
      root.render(
        createElement(FluentProvider, { theme: webLightTheme }, createElement(BrowseStrip, next)),
      )
    })
  }
  await update(props)
  return update
}

async function press(key: string) {
  const list = document.querySelector('[role="listbox"]')
  assert.ok(list, "Browse strip must expose its option list")
  await act(async () => {
    list.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }))
  })
}

function button(label: string) {
  const element = document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
  assert.ok(element, `Missing button: ${label}`)
  return element
}

function currentOption() {
  const option = document.querySelector('[role="option"][aria-selected="true"]')
  assert.ok(option, "Browse strip must mark the current schematic")
  return option
}

test("keyboard navigation selects sorted edges without wrapping past either end", async () => {
  const paths = ["a.litematic", "b.litematic", "c.litematic"]
  const selected: number[] = []
  const props = (index: number): BrowseStripProps => ({
    onNext: () => {},
    onPrevious: () => {},
    onSelect: (next) => selected.push(next),
    selection: { paths, index },
  })
  const update = await mount(props(0))

  assert.equal(button("Previous schematic").disabled, true)
  assert.equal(button("Next schematic").disabled, false)
  await press("ArrowLeft")
  assert.deepEqual(selected, [], "ArrowLeft on the first schematic must not wrap")

  await press("ArrowRight")
  assert.deepEqual(selected, [1])

  await update(props(1))
  await press("End")
  assert.deepEqual(selected, [1, 2], "End must select the last sorted schematic")

  await update(props(2))
  assert.equal(button("Next schematic").disabled, true)
  await press("ArrowRight")
  assert.deepEqual(selected, [1, 2], "ArrowRight on the last schematic must not wrap")

  await press("Home")
  assert.deepEqual(selected, [1, 2, 0], "Home must select the first sorted schematic")
})

test("a thumbnail finishing for an old selection does not fill the current schematic", async () => {
  const requests = new Map<string, (url: string | null) => void>()
  tauri.invoke.mockImplementation((command, args) => {
    assert.equal(command, "browse_thumbnail")
    const path =
      typeof args === "object" && args !== null && "path" in args && typeof args.path === "string"
        ? args.path
        : null
    assert.ok(path, "Thumbnail requests must name a path")
    const { promise, resolve } = Promise.withResolvers<string | null>()
    requests.set(path, resolve)
    return promise
  })
  const accept = async (path: string, url: string) => {
    const resolve = requests.get(path)
    assert.ok(resolve, `No thumbnail request for ${path}`)
    await act(async () => resolve(url))
  }
  const paths = ["a.litematic", "b.litematic"]
  const props = (index: number): BrowseStripProps => ({
    onNext: () => {},
    onPrevious: () => {},
    onSelect: () => {},
    selection: { paths, index },
  })
  const update = await mount(props(0))

  assert.deepEqual(
    [...requests.keys()].sort(),
    ["a.litematic", "b.litematic"],
    "Visible schematics must request thumbnails",
  )

  await update(props(1))
  assert.equal(currentOption().querySelector("img"), null)

  await accept("a.litematic", "data:image/png;base64,a")
  assert.equal(
    currentOption().querySelector("img"),
    null,
    "A late thumbnail for the previous schematic must not replace the current one",
  )

  await accept("b.litematic", "data:image/png;base64,b")
  assert.equal(
    currentOption().querySelector("img")?.getAttribute("src"),
    "data:image/png;base64,b",
    "The current schematic must show its own thumbnail",
  )
})
