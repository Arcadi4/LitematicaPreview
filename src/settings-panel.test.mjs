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

const notices = [
  "About decoder memory limits",
  "About chunk separation",
  "About multithreading",
  "About conservative memory scheduling",
]

test("worker count remains an accessible numeric input followed by the threads unit", () => {
  const markup = render({ ...defaultPreviewSettings, threadCount: 6 })
  const workers = input(markup, "Worker threads")
  assert.match(workers, /value="6"/)
  assert.doesNotMatch(workers, /\bdisabled(?:=|\s|>)/)
  assert.ok(markup.indexOf('aria-label="Worker threads"') > markup.indexOf("Enable multithreading"))
  assert.match(markup, /<span[^>]*>threads<\/span>/)
  assert.doesNotMatch(markup, />Worker threads</)
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
  const buttons = [...markup.matchAll(/<button\b[^>]*>/g)].map(([tag]) => tag)
  for (const label of notices) {
    const button = buttons.find((tag) => tag.includes(`aria-label="${label}"`))
    assert.ok(button, `Missing help button: ${label}`)
    assert.match(button, /aria-describedby="[^"]+"/)
    assert.doesNotMatch(button, /\bdisabled(?:=|\s|>)/)
  }
  assert.match(
    markup,
    /<p[^>]*>Changes are saved automatically and apply the next time you open a schematic\. The current preview or load is not changed\.<\/p>/,
  )
  assert.doesNotMatch(markup, /aria-label="When preview settings apply"/)
})

test("paired controls still refer to their setting-specific description", () => {
  const markup = render()
  assert.match(input(markup, "Worker threads"), /aria-describedby="thread-count-description"/)
  assert.match(
    input(markup, "Decoder memory limit (MB)"),
    /aria-describedby="memory-limit-description"/,
  )
  assert.match(
    input(markup, "Chunk size (blocks per side)"),
    /aria-describedby="chunk-size-description"/,
  )
})
