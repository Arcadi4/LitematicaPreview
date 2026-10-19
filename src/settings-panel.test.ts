import assert from "node:assert/strict"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"
import { test } from "vite-plus/test"
import type { Bootstrap } from "./app-model"
import { defaultPreviewSettings, type PreviewSettings } from "./preview-settings"
import { SettingsPanel } from "./settings-panel"

const bootstrap: Bootstrap = {
  extensions: [],
  demos: [],
  initialPath: null,
  version: "0.3.0",
  requestId: 0,
  maxWorkerThreads: 8,
  viewportError: null,
  texturePacks: { packs: [], selected: null },
  texturePackError: null,
}

function render(
  settings: PreviewSettings = defaultPreviewSettings,
  available: Bootstrap | null = bootstrap,
) {
  return renderToStaticMarkup(
    createElement(SettingsPanel, { settings, bootstrap: available, onChange: () => {} }),
  )
}

function input(markup: string, label: string) {
  const element = [...markup.matchAll(/<input\b[^>]*>/g)].find(([tag]) =>
    tag.includes(`aria-label="${label}"`),
  )
  assert.ok(element, `Missing accessible input: ${label}`)
  return element[0]
}

test("worker count renders the selected thread count in an enabled numeric input", () => {
  const workers = input(render({ ...defaultPreviewSettings, threadCount: 6 }), "Worker threads")
  assert.match(workers, /value="6"/)
  assert.doesNotMatch(workers, /\bdisabled(?:=|\s|>)/)
})

test("disabled preview controls retain enabled and described help buttons", () => {
  const markup = render(
    {
      ...defaultPreviewSettings,
      chunkingEnabled: false,
      multithreadingEnabled: false,
    },
    null,
  )
  assert.match(input(markup, "Worker threads"), /\bdisabled=""/)
  const described = [...markup.matchAll(/<button\b[^>]*aria-describedby="([^"]+)"[^>]*>/g)].map(
    ([tag, id]) => ({ tag, id }),
  )
  assert.ok(described.length > 0, "Settings must expose described help buttons")
  for (const { tag } of described) assert.doesNotMatch(tag, /\bdisabled(?:=|>|\s)/)
  // Distinct ids ensure each help description stays linked to its own control.
  assert.equal(new Set(described.map(({ id }) => id)).size, described.length)
})
