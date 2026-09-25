import assert from "node:assert/strict"
import { test } from "vite-plus/test"
import { savedPreviewSettings } from "./App.tsx"

const key = "litematica-preview-settings"

test("saved scheduling preference retains its actual behavior across the inverted control", () => {
  const previousStorage = globalThis.localStorage
  const saved = new Map()
  globalThis.localStorage = {
    getItem: (name) => saved.get(name) ?? null,
    setItem: (name, value) => saved.set(name, value),
  }
  try {
    assert.equal(savedPreviewSettings().conservativeMemoryScheduling, true)
    assert.equal(savedPreviewSettings().multithreadingEnabled, true)
    assert.equal(savedPreviewSettings().threadCount, 4)
    saved.set(key, JSON.stringify({ multithreadingEnabled: false, threadCount: 2 }))
    assert.equal(savedPreviewSettings().multithreadingEnabled, false)
    assert.equal(savedPreviewSettings().threadCount, 2)
    saved.delete(key)
    for (const [oldValue, conservative] of [
      [false, true],
      [true, false],
    ]) {
      saved.set(
        key,
        JSON.stringify({
          chunkingEnabled: true,
          multithreadingEnabled: true,
          speedFirst: oldValue,
        }),
      )
      const migrated = savedPreviewSettings()
      assert.equal(migrated.conservativeMemoryScheduling, conservative)
      assert.equal(migrated.multithreadingEnabled, true)
      saved.set(key, JSON.stringify(migrated))
      assert.equal(savedPreviewSettings().conservativeMemoryScheduling, conservative)
      assert.equal(JSON.parse(saved.get(key)).speedFirst, undefined)
    }
    saved.set(key, JSON.stringify({ conservativeMemoryScheduling: false, speedFirst: false }))
    assert.equal(savedPreviewSettings().conservativeMemoryScheduling, false)
  } finally {
    if (previousStorage === undefined) delete globalThis.localStorage
    else globalThis.localStorage = previousStorage
  }
})
