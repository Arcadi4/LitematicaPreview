import { Button, Switch } from "@fluentui/react-components"
import {
  Add20Regular,
  ArrowDown20Regular,
  ArrowUp20Regular,
  Delete20Regular,
} from "@fluentui/react-icons"
import { useState } from "react"
import type { TexturePackState } from "./app-model"

export interface TexturePacksPanelProps {
  busy: boolean
  onImport: () => void
  onRemove: (id: string) => void
  onSetEnabled: (id: string, enabled: boolean) => void
  onSetOrder: (ids: string[]) => void
  state: TexturePackState
}

export function TexturePacksPanel({
  busy,
  onImport,
  onRemove,
  onSetEnabled,
  onSetOrder,
  state,
}: TexturePacksPanelProps) {
  const [confirming, setConfirming] = useState<string | null>(null)

  const move = (index: number, offset: -1 | 1) => {
    const target = index + offset
    if (target < 0 || target >= state.packs.length) return
    const ids = state.packs.map((pack) => pack.id)
    const reordered = [...ids]
    reordered[index] = ids[target]
    reordered[target] = ids[index]
    onSetOrder(reordered)
  }

  return (
    <div className="flex flex-col gap-5">
      <p className="m-0 leading-relaxed">
        Top packs take priority; missing textures, models, and block states fall back to lower
        packs, then vanilla. Changes reload the current preview.
      </p>
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-sm font-semibold">Highest priority</span>
          <Button icon={<Add20Regular />} disabled={busy} onClick={onImport}>
            {busy ? "Working…" : "Add texture packs…"}
          </Button>
        </div>
        {state.packs.length === 0 ? (
          <p className="m-0 text-sm text-muted">
            No texture packs are saved. The bundled vanilla resources are used.
          </p>
        ) : (
          <ol
            aria-label="Texture packs, highest priority first"
            className="m-0 flex list-none flex-col gap-2 p-0"
          >
            {state.packs.map((pack, index) => (
              <li
                key={pack.id}
                className="flex flex-wrap items-center gap-2 rounded-md border border-border bg-surface-secondary px-3 py-2"
              >
                <span aria-hidden="true" className="w-5 text-sm tabular-nums text-muted">
                  {index + 1}
                </span>
                <Switch
                  className="min-w-48 flex-1"
                  checked={pack.enabled}
                  disabled={busy}
                  label={pack.name}
                  onChange={(_, data) => onSetEnabled(pack.id, data.checked)}
                />
                <Button
                  appearance="subtle"
                  size="small"
                  icon={<ArrowUp20Regular />}
                  aria-label={`Move ${pack.name} up`}
                  disabled={busy || index === 0}
                  onClick={() => move(index, -1)}
                />
                <Button
                  appearance="subtle"
                  size="small"
                  icon={<ArrowDown20Regular />}
                  aria-label={`Move ${pack.name} down`}
                  disabled={busy || index === state.packs.length - 1}
                  onClick={() => move(index, 1)}
                />
                {confirming === pack.id ? (
                  <>
                    <Button
                      appearance="primary"
                      size="small"
                      disabled={busy}
                      onClick={() => {
                        setConfirming(null)
                        onRemove(pack.id)
                      }}
                    >
                      Remove “{pack.name}”
                    </Button>
                    <Button size="small" disabled={busy} onClick={() => setConfirming(null)}>
                      Cancel
                    </Button>
                  </>
                ) : (
                  <Button
                    appearance="subtle"
                    size="small"
                    icon={<Delete20Regular />}
                    aria-label={`Remove ${pack.name} from saved packs`}
                    disabled={busy}
                    onClick={() => setConfirming(pack.id)}
                  />
                )}
              </li>
            ))}
          </ol>
        )}
        <div className="rounded-md border border-border bg-surface-secondary px-3 py-2 text-sm">
          <p className="m-0 font-semibold">Bundled vanilla resources</p>
          <p className="m-0 text-muted">Always enabled. Lowest priority fallback.</p>
        </div>
      </div>
    </div>
  )
}
