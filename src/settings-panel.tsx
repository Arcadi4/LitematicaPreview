import { Field, Slider, SpinButton, Switch } from "@fluentui/react-components"
import type { Bootstrap } from "./app-model"
import { chunkSizes, type PreviewSettings } from "./preview-settings"

export interface SettingsPanelProps {
  bootstrap: Bootstrap | null
  onChange: (patch: Partial<PreviewSettings>) => void
  settings: PreviewSettings
}

/** Reads a spin button value typed as text into a validated integer, or null. */
function spinValue(value: number | null, displayValue: string | undefined) {
  if (value !== null) return value
  if (!displayValue || !/^\d+$/.test(displayValue)) return null
  return Number(displayValue)
}

export function SettingsPanel({ bootstrap, onChange, settings }: SettingsPanelProps) {
  const maxThreads = Math.max(2, bootstrap?.maxWorkerThreads ?? 2)
  return (
    <div className="flex flex-col gap-5 max-h-[60vh] overflow-y-auto">
      <p className="m-0 leading-relaxed">
        Changes are saved automatically and apply the next time you open a schematic. The current
        preview or load is not changed.
      </p>
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between gap-4">
          <Switch
            label="Limit decoder memory"
            checked={settings.memoryLimitEnabled}
            aria-describedby="memory-limit-description"
            onChange={(_, data) => onChange({ memoryLimitEnabled: data.checked })}
          />
          <div className="ml-auto flex items-center gap-2">
            <SpinButton
              value={settings.memoryLimitMB}
              min={2048}
              max={8192}
              step={1}
              disabled={!settings.memoryLimitEnabled}
              aria-label="Decoder memory limit (MB)"
              aria-describedby="memory-limit-description"
              className="w-32"
              onChange={(_, data) => {
                const value = spinValue(data.value ?? null, data.displayValue)
                if (value !== null && Number.isInteger(value) && value >= 2048 && value <= 8192)
                  onChange({ memoryLimitMB: value })
              }}
            />
            <span className="text-sm">MB</span>
          </div>
        </div>
        <p id="memory-limit-description" className="m-0 text-sm leading-relaxed text-muted">
          Limits the isolated decoder process, not graphics or total application memory. If the
          decoder stops while this limit is enabled, loading returns to Home and shows an error; the
          limit may be involved, but a crash cannot confirm it was reached. Disabling the limit may
          exhaust system memory.
        </p>
      </div>
      <div className="flex flex-col gap-3">
        <Switch
          label="Separate geometry into chunks"
          checked={settings.chunkingEnabled}
          disabled={settings.multithreadingEnabled}
          aria-describedby="chunk-size-description"
          onChange={(_, data) => onChange({ chunkingEnabled: data.checked })}
        />
        <Field label={`Chunk size (blocks per side): ${settings.chunkSize}`}>
          <Slider
            className="chunk-size-slider mb-8"
            min={0}
            max={chunkSizes.length - 1}
            step={1}
            aria-label="Chunk size (blocks per side)"
            value={chunkSizes.indexOf(settings.chunkSize)}
            disabled={!settings.chunkingEnabled}
            rail={{
              className: "chunk-size-rail",
              children: chunkSizes.map((size, index) => (
                <span
                  key={size}
                  aria-hidden="true"
                  className="chunk-size-mark"
                  style={{ left: `${(index / (chunkSizes.length - 1)) * 100}%` }}
                >
                  <span className="chunk-size-mark-dot" />
                  <span className="chunk-size-mark-label">{size}</span>
                </span>
              )),
            }}
            onChange={(_, data) => onChange({ chunkSize: chunkSizes[data.value] })}
          />
        </Field>
        <p id="chunk-size-description" className="m-0 text-sm leading-relaxed text-muted">
          Smaller chunks reduce peak meshing memory and allow cancellation between chunks. Disabling
          chunk separation increases peak memory use and cancellation latency.
        </p>
      </div>
      <div className="flex flex-col gap-3">
        <Switch
          label="Enable multithreading"
          checked={settings.multithreadingEnabled}
          disabled={!settings.chunkingEnabled || !bootstrap || bootstrap.maxWorkerThreads < 2}
          aria-describedby="thread-count-description"
          onChange={(_, data) => onChange({ multithreadingEnabled: data.checked })}
        />
        <Field label="Worker threads">
          <SpinButton
            value={settings.threadCount}
            min={2}
            max={maxThreads}
            step={1}
            disabled={!settings.multithreadingEnabled}
            aria-label="Worker threads"
            aria-describedby="thread-count-description"
            className="w-32"
            onChange={(_, data) => {
              const value = spinValue(data.value ?? null, data.displayValue)
              if (
                value !== null &&
                Number.isInteger(value) &&
                value >= 2 &&
                value <= (bootstrap?.maxWorkerThreads ?? 1)
              )
                onChange({ threadCount: value })
            }}
          />
        </Field>
        <Switch
          label="Conservative memory scheduling"
          checked={settings.conservativeMemoryScheduling}
          disabled={!settings.multithreadingEnabled}
          aria-describedby="conservative-memory-description"
          onChange={(_, data) => onChange({ conservativeMemoryScheduling: data.checked })}
        />
        <p id="conservative-memory-description" className="m-0 text-sm leading-relaxed text-muted">
          Disabled by default. Enable it to reduce concurrent mesh work and the number of queued
          batches and upload pages; this may reduce decoding speed. Leaving it disabled uses the
          selected worker count more aggressively and may use more memory. A separate decoder memory
          limit remains effective in either mode.
        </p>
        <p id="thread-count-description" className="m-0 text-sm leading-relaxed text-muted">
          Requires chunk separation and at least two available logical processors. Uses up to{" "}
          {bootstrap?.maxWorkerThreads ?? 1} workers for parallel decoding, meshing and upload
          preparation. Memory-first scheduling may use fewer workers; additional working buffers can
          still increase peak memory. GPU submission remains on the main thread.
        </p>
      </div>
    </div>
  )
}
