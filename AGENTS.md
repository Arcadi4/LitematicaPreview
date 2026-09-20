# Litematica Preview

Windows x64 desktop app for offline Minecraft schematic and structure viewing.
- `App/`: Tauri 2 desktop application with a Rust host (`App/src-tauri/`) and Fluent UI React v9 frontend (`App/src/`). Uses an on-demand WebGL 2 renderer with `gl-matrix`.
- `Native/`: Rust library (`litematica_preview_native`) wrapping Nucleation 0.10.14 for decoding and meshing.
- `Assets/`: Tracked inputs including default resource pack (`Assets/pack.zip`) and application icons.
- `Fixtures/`: Seven demo schematics in `Demos/` and minimal format fixtures in `Formats/`.
- `scripts/`: PowerShell build scripts (`build.ps1`) and NSIS installer configuration (`installer-hooks.nsh`).

Seven supported formats: `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.nusn` (defined once in Rust host `EXTENSIONS` in `App/src-tauri/src/main.rs`).

## Setup and Prerequisites

- Package manager: `pnpm` (v12, see `devEngines` in `App/package.json`).
- Node.js: Node.js 24 LTS.
- Rust: Current stable Rust toolchain with `x86_64-pc-windows-msvc` target for Windows builds.
- Windows desktop builds: Visual Studio C++ Build Tools (Desktop development with C++, x64 MSVC, Windows SDK) and WebView2 Evergreen Runtime.
- Install frontend dependencies:
  ```bash
  pnpm --prefix App install --frozen-lockfile
  ```

## Development Workflow

- Run the desktop app with hot-reloading (Windows):
  ```bash
  pnpm --prefix App exec tauri dev
  ```
- Run frontend development server (Vite+ on port 1420):
  ```bash
  pnpm --prefix App run dev
  ```
- Build frontend assets (`tsc --noEmit && vp build`):
  ```bash
  pnpm --prefix App run build
  ```
- Preview built frontend:
  ```bash
  pnpm --prefix App run preview
  ```

## Testing and Quality Checks

- Check frontend formatting:
  ```bash
  pnpm --prefix App run format -- --check
  ```
- Format frontend code (oxfmt, semi: false):
  ```bash
  pnpm --prefix App run format
  ```
- Check Rust formatting:
  ```bash
  cargo fmt --manifest-path Native/Cargo.toml -- --check
  cargo fmt --manifest-path App/src-tauri/Cargo.toml -- --check
  ```
- Native decoder and mesher tests:
  ```bash
  cargo test --manifest-path Native/Cargo.toml --release --locked
  ```
- Rust Tauri host compile check:
  ```bash
  cargo check --manifest-path App/src-tauri/Cargo.toml --all-targets --locked
  ```
- Rust Tauri host unit tests:
  ```bash
  cargo test --manifest-path App/src-tauri/Cargo.toml --release --locked
  ```

## Build and Packaging

- Build portable Windows app (`artifacts/win-x64/`):
  ```powershell
  ./scripts/build.ps1
  ```
- Build portable app and NSIS setup installer (`artifacts/*-setup.exe`):
  ```powershell
  ./scripts/build.ps1 -Installer
  ```
- Do not report Windows runtime, installer, or benchmark results from a Mac or non-Windows environment.

## Ownership and Constraints

- Rust owns decoded previews until binary IPC serialization finishes. The WebView
  uploads typed-array views to WebGL and releases superseded CPU/GPU resources.
- The native resource pack is cached, with one active decode/mesh call per process.
  Cancel queued work and dispose stale results; Nucleation itself is not cancellable.
- Keep opaque, cutout, and transparent parts separate. Greedy materials use their
  own repeating textures. The atlas is shared and clamped. Never flatten them.
- Include greedy parts in triangle counts and bounds. Nucleation 0.10.14's
  `total_triangles()` and `is_empty()` omit them.
- Frame geometry bounds rather than chunk-padded schematic dimensions.
- Render on load/resize/input. No perpetual frame timer, decorative animations,
  compatibility shell, or new general-purpose service layer. Tauri's local WebView
  and IPC bridge are permitted; Fluent UI is the desktop interface.
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
