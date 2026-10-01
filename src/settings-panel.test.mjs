import assert from "node:assert/strict"
import { createElement } from "react"
import { renderToStaticMarkup } from "react-dom/server"
import { test } from "vite-plus/test"
import { defaultPreviewSettings } from "./preview-settings.ts"
import { SettingsPanel } from "./settings-panel.tsx"

const bootstrap = {
  extensions: [],
  demos: [],
  initialPath: null,
  version: "0.3.0",
  requestId: 0,
  maxWorkerThreads: 8,
}

function render(settings = defaultPreviewSettings, available = bootstrap) {
  return renderToStaticMarkup(
    createElement(SettingsPanel, { settings, bootstrap: available, onChange: () => {} }),
  )
}

function input(markup, label) {
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
      memoryLimitEnabled: false,
      chunkingEnabled: false,
      multithreadingEnabled: false,
    },
    null,
  )
  assert.match(input(markup, "Worker threads"), /\bdisabled=""/)
  assert.match(input(markup, "Decoder memory limit (MB)"), /\bdisabled=""/)
  const described = [...markup.matchAll(/<button\b[^>]*aria-describedby="([^"]+)"[^>]*>/g)].map(
    ([tag, id]) => ({ tag, id }),
  )
  assert.ok(described.length > 0, "Settings must expose described help buttons")
  for (const { tag } of described) assert.doesNotMatch(tag, /\bdisabled(?:=|\s|>)/)
  // A copied description id would attach one setting's help text to another's control.
  assert.equal(new Set(described.map(({ id }) => id)).size, described.length)
})
