# Litematica Preview

Windows x64 desktop app for offline Minecraft schematic and structure viewing (Tauri 2 + Rust wgpu renderer + React 19 Fluent UI v9 frontend).

- **Architecture**: Host (`src-tauri/`) embeds a Direct3D 12 wgpu renderer drawing directly into the main window beneath a transparent WebView2 overlay (`src/`). The frontend manages UI state and controls; it renders zero canvas pixels.
- **Shared Core**: Geometry decoding, caching, and meshing logic live in `litematica-preview-core` (consumed from crates.io; override via `LP_CORE_PATH=../litematica-preview-core`).
- **Formats**: `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.nusn` (defined in `src-tauri/src/main.rs`).
- **Assets & Fixtures**: Default pack in `Assets/pack.zip`. Test fixtures in `Fixtures/Demos/` and `Fixtures/Formats/`.

## Commands

Package manager is `pnpm` (v12). Vite+ (`vp`) powers the web toolchain; `tsx` runs project scripts.

| Task                    | Command                                                                  | Notes                                             |
| ----------------------- | ------------------------------------------------------------------------ | ------------------------------------------------- |
| Install dependencies    | `pnpm install --frozen-lockfile`                                         |                                                   |
| App dev (hot reload)    | `pnpm run tauri:dev`                                                     | Windows                                           |
| Web dev server          | `pnpm run dev`                                                           | Port 1420                                         |
| Web build & preview     | `pnpm run build && pnpm run preview`                                     | `tsc --noEmit && vp build`                        |
| Full CI check           | `pnpm run check`                                                         | Runs lint, format, build, frontend and host tests |
| Lint & format           | `pnpm run lint` / `pnpm run format`                                      | Oxlint & Oxfmt (`vp fmt`)                         |
| Frontend tests          | `pnpm run test`                                                          | Vitest via `vp test run`                          |
| Rust checks             | `pnpm run check:host`                                                    | `cargo check --all-targets --locked`              |
| Rust tests              | `pnpm run test:host`                                                     | `cargo test --locked`                             |
| Rust format check       | `pnpm run format:rust`                                                   | `cargo fmt -- --check`                            |
| Test against local core | `LP_CORE_PATH=<path> pnpm run test:host`                                 | Run `pnpm run resolve:host` to revert             |
| Version check / bump    | `pnpm run version:check` / `pnpm run version:bump [patch\|minor\|major]` | Syncs package.json, Cargo, and tauri configs      |
| Windows package build   | `pnpm run build:win`                                                     | Creates portable zip & NSIS setup in `artifacts/` |
| Release checksums       | `pnpm run checksums`                                                     | Generates `artifacts/SHA256SUMS.txt`              |

## Architecture & Operational Constraints

- **Renderer & Geometry**: Shared core generates upload-ready geometry and RGBA textures. Host joins batch segmented buffers once and uploads directly to wgpu. Batches use 24-byte compact vertices or 28-byte float positions (>256 block spans). One geometry buffer per batch; slice vertex/index ranges per part.
- **Materials**: Never flatten opaque, cutout, and transparent parts. Greedy materials use dedicated repeating textures; atlas is shared and clamped. Include greedy parts in triangle counts and bounds.
- **Framing**: Frame actual geometry bounds, not chunk-padded schematic bounds. Render on demand (load/resize/input); no continuous animation frames.
- **Decoder Worker**: Dedicated isolated worker process caches resource packs and serializes decode/mesh jobs. Enforced 2 GiB memory limit on Windows; kills on host exit. Worker crashes must show an error dialog with home navigation recovery.
- **Registry & Windows Integration**: Register file associations under `HKCU`. Quote executable and file paths separately. Never overwrite Windows `UserChoice` or unregister another installed app's ProgID.
- **Host Verification**: Windows hardware required to verify Direct3D 12 composite swapchain, transparent WebView2 layering, pointer mapping, stage-rectangle framing, DPI scaling across monitors, streaming cancellations, and device-loss handling. A viewport change must also clear: no startup flash before the first presented frame; resize, snap, maximize and fullscreen re-present without stretch or ghosting; focus, Tab and arrow keys reach the canvas; native dialogs sit above the main window with correct modality; minimize, restore, occlusion and alt-tab suspend and resume presenting; long-run semi-transparent DOM overlays still composite over live GPU content; note the exact WebView2 Evergreen runtime version tested. Do not report Windows runtime verification from non-Windows environments.
- Never add JavaScript files (including `.mjs`, `.cjs`). Always use TypeScript.

## Comments

Code explains itself: a reader learns what a function does from its name, parameters, and types, not from prose above the call site. The default is no comment.

- Never restate the code, narrate a body, or repeat a callee's documentation at the call site. Rename, extract, or move the check instead.
- One home per fact: a rationale shared by a type and its methods, or by peers delegating to one helper, lives at that common owner; peers and call sites link to it (`[`Item`]` in Rust, `{@link item}` in TS) instead of re-deriving it.
- Doc comments describe the item's contract — preconditions, postconditions, edge cases, why it exists — never its body.
- Never record replaced implementations, refactor history, or removed files; that belongs in the commit message. A reference to code that no longer exists is a bug in the comment.
- Write what cannot go stale: name a list instead of counting it, state a limit or default once on the item that owns it, link a section by anchor or item rather than quoting it, and keep example names fake.
- Concise and structured: a list or a labelled line beats a paragraph, one word per concept, and values are spelled the way the code spells them.
- A comment earns its place for a hidden invariant, a workaround for a known bug, or a deliberate exception to the surrounding pattern — then it is one line, so the next reader does not take it for an oversight. A behavioral claim is a claim: verify it or leave it out.
