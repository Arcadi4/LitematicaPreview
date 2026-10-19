import { Button, Dropdown, Field, Option } from "@fluentui/react-components"
import { Add20Regular, Delete20Regular } from "@fluentui/react-icons"
import { useState } from "react"
import type { TexturePackState } from "./app-model"

export interface TexturePacksPanelProps {
  busy: boolean
  onImport: () => void
  onRemove: (id: string) => void
  onSelect: (id: string | null) => void
  state: TexturePackState
}

export function TexturePacksPanel({
  busy,
  onImport,
  onRemove,
  onSelect,
  state,
}: TexturePacksPanelProps) {
  const [confirming, setConfirming] = useState<string | null>(null)
  const selected = state.packs.find((pack) => pack.id === state.selected)
  const removeConfirmed = confirming !== null && confirming === selected?.id

  return (
    <div className="flex flex-col gap-5">
      <p className="m-0 leading-relaxed">
        Use a Minecraft Java resource-pack ZIP for block textures. Imported packs are copied into
        the app data folder; the original ZIP is never changed. Texture pack changes apply to the
        current preview immediately.
      </p>
      <Field
        label="Texture pack"
        hint="Packs layer over the bundled vanilla resources, so missing textures stay visible."
      >
        <Dropdown
          aria-label="Texture pack"
          className="min-w-64"
          disabled={busy}
          selectedOptions={[state.selected ?? "vanilla"]}
          value={selected?.name ?? "Vanilla (bundled)"}
          onOptionSelect={(_, data) => {
            if (data.optionValue === undefined) return
            onSelect(data.optionValue === "vanilla" ? null : data.optionValue)
          }}
        >
          <Option value="vanilla">Vanilla (bundled)</Option>
          {state.packs.map((pack) => (
            <Option key={pack.id} value={pack.id}>
              {pack.name}
            </Option>
          ))}
        </Dropdown>
      </Field>
      <div className="flex flex-wrap items-center gap-2">
        <Button icon={<Add20Regular />} disabled={busy} onClick={onImport}>
          {busy ? "Working…" : "Add texture packs…"}
        </Button>
        {selected &&
          (removeConfirmed ? (
            <>
              <Button
                appearance="primary"
                disabled={busy}
                onClick={() => {
                  setConfirming(null)
                  onRemove(selected.id)
                }}
              >
                Remove “{selected.name}”
              </Button>
              <Button disabled={busy} onClick={() => setConfirming(null)}>
                Cancel
              </Button>
            </>
          ) : (
            <Button
              icon={<Delete20Regular />}
              disabled={busy}
              onClick={() => setConfirming(selected.id)}
            >
              Remove saved copy
            </Button>
          ))}
      </div>
      {state.packs.length === 0 && (
        <p className="m-0 text-sm text-muted">
          No texture packs are saved yet. Add a Minecraft Java resource pack to preview your builds
          with its textures.
        </p>
      )}
    </div>
  )
}
