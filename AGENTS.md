# Litematica Preview

Windows x64 desktop app for offline Minecraft schematic and structure viewing. The root is both a pnpm workspace and a Cargo workspace.

- Root: Tauri 2 desktop application with a Rust host (`src-tauri/`) and Fluent UI React v9 frontend (`src/`). Rendering is a native Rust renderer (`src-tauri/src/render/`, wgpu with Direct3D 12) that paints into the main window beneath a transparent WebView2; `src/` handles no canvas pixels.
- The shared engine lives in https://github.com/Arcadi4/litematica-preview-core and is consumed from crates.io as `litematica-preview-core`. Set `LP_CORE_PATH` to a local checkout to build against unpublished core changes; `scripts/cargo.ts` applies the patch. LitematicaQL uses the same variable via its own `Core/cargo.sh`.
- `Assets/`: Tracked inputs including default resource pack (`Assets/pack.zip`) and application icons.
- `Fixtures/`: Seven demo schematics in `Demos/` and minimal format fixtures in `Formats/`.
- `scripts/`: Cross-platform TypeScript tooling (`build.ts`, `cargo.ts`, `checksums.ts`, `record-memory.ts`, `bump-version.ts`) and NSIS installer configuration (`installer.nsi`, `installer-hooks.nsh`).

Every repository task is a package script in `package.json`; there are no PowerShell entry points. Scripts run through `tsx` except `bump-version.ts`, which stays dependency-free so release tags can be validated before `pnpm install`.

Seven supported formats: `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.nusn` (defined once in Rust host `EXTENSIONS` in `src-tauri/src/main.rs`).

## Setup and Prerequisites

- Package manager: `pnpm` (v12, see `devEngines` in `package.json`).
- Node.js: Node.js 24 LTS.
- Rust: Current stable Rust toolchain with `x86_64-pc-windows-msvc` target for Windows builds.
- Windows desktop builds: Visual Studio C++ Build Tools (Desktop development with C++, x64 MSVC, Windows SDK) and WebView2 Evergreen Runtime.
- Install frontend dependencies:

  ```bash
  pnpm install --frozen-lockfile
  ```

## Development Workflow

- Run the desktop app with hot-reloading (Windows):

  ```bash
  pnpm run tauri:dev
  ```

- Run frontend development server (Vite+ on port 1420):

  ```bash
  pnpm run dev
  ```

- Build frontend assets (`tsc --noEmit && vp build`):

  ```bash
  pnpm run build
  ```

- Preview built frontend:

  ```bash
  pnpm run preview
  ```

## Testing and Quality Checks

- Run every frontend and Rust check (what CI runs):

  ```bash
  pnpm run check
  ```

- Check frontend formatting:

  ```bash
  pnpm run format:check
  ```

- Lint frontend code (oxlint):

  ```bash
  pnpm run lint
  ```

- Format frontend code (oxfmt, semi: false):

  ```bash
  pnpm run format
  ```

- Run frontend tests:

  ```bash
  pnpm run test
  ```

- Check Rust formatting:

  ```bash
  pnpm run format:rust
  ```

- Core decoder and mesher tests run in the core repository, not here:

  ```bash
  cd ../litematica-preview-core && cargo test --locked
  ```

- Build the host against a local core checkout:

  ```bash
  LP_CORE_PATH=../litematica-preview-core pnpm run resolve:host
  LP_CORE_PATH=../litematica-preview-core pnpm run test:host
  ```

  This rewrites `Cargo.lock` to point at that checkout. Run
  `pnpm run resolve:host` without the override to restore the published dependency
  before committing.

- Rust Tauri host compile check:

  ```bash
  pnpm run check:host
  ```

- Rust Tauri host unit tests:

  ```bash
  pnpm run test:host
  ```

- Check version consistency across all project locations (accepts an optional tag):

  ```bash
  pnpm run version:check [tag_or_version]
  ```

- Bump app versions (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, and the app entry in `Cargo.lock`), commit, and tag (defaults to `patch`). The shared engine has an independent version:

  ```bash
  pnpm run version:bump [patch | minor | major]
  ```

- Record host and decoder process memory to CSV until interrupted (never overwrites an existing recording):

  ```bash
  pnpm run record-memory -- --process LitematicaPreview --interval 200
  ```

## Build and Packaging

### Native renderer verification (Windows hardware required)

The development machine compiles the wgpu renderer for both `aarch64/x86_64`
host targets and `x86_64-pc-windows-msvc` (`cargo check -p litematica-preview`
and the same with `--target`); tests cover the camera math, wire protocol and
stream leasing. Nothing here proves the Windows runtime, because checks do not
link: before cutting a release, verify on real Windows:

1. Startup composites the swapchain beneath the transparent WebView2 layer and
   shows the welcome screen with no flash of a blank/black frame.
2. Both-direct3D-12 path: the adapter error path surfaces when GPUs lack DXGI
   12 support, with no WARP attempt and no silent fallback.
3. Layering: Fluent dialogs, menus, drag overlays, control hints and tooltips
   all stay interactive above the viewport; input does not steal camera focus.
4. Pointer mapping: left-drag orbits, right/middle drags pan, wheel zooms
   (deltaMode lines/pages), arrows/shift-pan, +/- zoom, F/Home re-fit.
5. DPI: move between monitors with different scale factors, snap/restore,
   maximize, fullscreen, and multi-monitor 4K/5K sizes; the camera stays
   fitted and the swapchain covers the window.
6. Streaming: load multi-generation reloads, cancel mid-stream (Esc), demos
   through `loadDemo`, huge fixtures; generation changes never leave GPU
   staging behind or swap in a stale model.
7. Color: the background matches the WebGL renderer's dark `#010102`-class
   tint; block colours/sRGB blending match the previous screenshots, blending
   in linear and encoding on store.
8. Device-loss/OutOfMemory handling: pulling the adapter, `ResetDevice`,
   OOM on a giant fixture - the app reports per the events and stays usable.
9. Long-run compositing: rapid window resize + streaming at the same time
   doesn't leak textures/buffers (Task Manager + GPU memory).

Known open risks (record explicitly with a fix plan before shipping):

- `renderer.resize` reconfiguration happens on Resized only; a
  `ScaleFactorChanged` with no resize can leave the camera's CSS-px scale
  stale until the next resize.
- Blending/clear color parity with the WebGL renderer was recorded from GL
  semantics; a first visual diff on Windows may require adjusting the sRGB
  clear-color expansion in `render::context`.
- Device-loss recovery is minimal (frames fail with `SurfaceLost` and the app
  survives); a full device-rebuild path is not implemented.

- Build the Windows x64 release packages on Windows: the portable app (`artifacts/win-x64/`, `artifacts/*-portable.zip`) and the NSIS setup (`artifacts/*-setup.exe`):

  ```bash
  pnpm run build:win
  ```

- Write `artifacts/SHA256SUMS.txt` for the generated release packages:

  ```bash
  pnpm run checksums
  ```

- Do not report Windows runtime, installer, or benchmark results from a Mac or non-Windows environment.

## Ownership and Constraints

- The shared core produces validated upload-ready geometry and first-use RGBA
  textures. The host joins each batch's segmented buffers once and uploads
  them into the native wgpu renderer without quantizing attributes, rebasing
  indices, or caching texture pixels in the DOM layer. The renderer owns GPU
  resource lifetimes and releases superseded CPU/GPU resources.
- Batches use 24-byte compact vertices or 28-byte absolute float positions for
  spans above 256 blocks. Draw with the matching pipeline family for each
  part's `vertexStride`; part uniforms carry origin/extent so the shader
  applies the identity transform for the float layout while keeping actual
  bounds for framing. One geometry buffer per batch serves every part; slice
  buffers per part for vertex and index ranges.
- Bounded StreamQueue backpressure, generation cancelling and lease drains
  still govern production, and the host consumer attaches to the renderer
  within them. Core owns content validation.
- A persistent isolated decoder process caches the resource pack and serializes
  decode/mesh calls. Windows limits it to 2 GiB and kills it with the host.
  Cancel queued work and dispose stale results; Nucleation itself is not cancellable.
  Native worker crashes must surface in an error dialog with home recovery.
- Keep opaque, cutout, and transparent parts separate. Greedy materials use their
  own repeating textures. The atlas is shared and clamped. Never flatten them.
- Include greedy parts in triangle counts and bounds. Nucleation 0.10.14's
  `total_triangles()` and `is_empty()` omit them.
- Frame geometry bounds rather than chunk-padded schematic dimensions.
- Render on load/resize/input. No perpetual frame timer, decorative animations,
  compatibility shell, or new general-purpose service layer. Tauri's local
  WebView and IPC bridge supply the Fluent interface; the swapchain paints the
  schematic under a transparent WebView2 layer.
- Preserve Nucleation's exact version pin, bounded decoding, Java binary NBT
  fallback, and brace-state SNBT normalization until real fixtures justify changes.
- Register under HKCU. Quote executable and filename separately. Never overwrite
  Windows UserChoice or unregister another installed copy's ProgID.
- `Assets/pack.zip`, fixtures, icons, and license texts are tracked inputs.
  No build requires the sibling macOS repository. Node.js is a build-time dependency,
  not a runtime dependency; the shipped app requires WebView2, not .NET.

<!--VITE PLUS START-->

# Using Vite+, the Unified Toolchain for the Web

This project is using Vite+, a unified toolchain built on top of Vite, Rolldown, Vitest, tsdown, Oxlint, Oxfmt, and Vite Task. Vite+ wraps runtime management, package management, and frontend tooling in a single global CLI called `vp`. Vite+ is distinct from Vite, and it invokes Vite through `vp dev` and `vp build`. Run `vp help` to print a list of commands and `vp <command> --help` for information about a specific command.

Docs are local at `node_modules/vite-plus/docs` or online at <https://viteplus.dev/guide/>.

## Built-in Commands vs Scripts

`vp <name>` runs a built-in command. `vp run <name>` runs a `package.json` script or a `vite.config.ts` task. Scripts cannot overwrite built-ins, so `vp dev` and `vp run dev` may do different things. Check `package.json` and `vite.config.ts` first, and run `vp run <name>` when the project defines a script or task with that name.

## Tool Versions

Run `vp toolchain` to show versions and relationships in the active Vite+
release. Add a tool name to select part of the graph. For example, run
`vp toolchain vite`. Use `--global` to ignore the local `vite-plus` package. Use
`vp why <package>` to show the package-manager dependency graph.

## Review Checklist

- [ ] Run `vp install` after pulling remote changes and before getting started.
- [ ] Run `vp check` and `vp test` to format, lint, type check and test changes.
- [ ] Check if there are `vite.config.ts` tasks or `package.json` scripts necessary for validation, run via `vp run <script>`.
- [ ] If setup, runtime, or package-manager behavior looks wrong, run `vp env doctor` and include its output when asking for help.

<!--VITE PLUS END-->
