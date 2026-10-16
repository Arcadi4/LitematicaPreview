import assert from "node:assert/strict"
import { afterEach, test, vi } from "vite-plus/test"
import { savedPreviewSettings } from "./preview-settings"

const key = "litematica-preview-settings"

afterEach(() => vi.unstubAllGlobals())

test("saved scheduling preference retains its actual behavior across the inverted control", () => {
  const saved = new Map<string, string>()
  vi.stubGlobal("localStorage", {
    getItem: (name: string) => saved.get(name) ?? null,
    setItem: (name: string, value: string) => {
      saved.set(name, value)
    },
  })
  assert.equal(savedPreviewSettings().conservativeMemoryScheduling, false)
  assert.equal(savedPreviewSettings().multithreadingEnabled, true)
  assert.equal(savedPreviewSettings().threadCount, 4)
  saved.set(key, JSON.stringify({ multithreadingEnabled: false, threadCount: 2 }))
  assert.equal(savedPreviewSettings().multithreadingEnabled, false)
  assert.equal(savedPreviewSettings().threadCount, 2)
  saved.delete(key)
  saved.set(key, JSON.stringify({ multithreadingEnabled: true, threadCount: 4 }))
  assert.equal(savedPreviewSettings().conservativeMemoryScheduling, false)
  saved.delete(key)
  for (const [oldValue, conservative] of [
    [false, true],
    [true, false],
  ] as const) {
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
    const persisted = saved.get(key)
    assert.ok(persisted, "Migrated settings must be persisted")
    assert.equal(JSON.parse(persisted).speedFirst, undefined)
  }
  saved.set(key, JSON.stringify({ conservativeMemoryScheduling: false, speedFirst: false }))
  assert.equal(savedPreviewSettings().conservativeMemoryScheduling, false)
})
