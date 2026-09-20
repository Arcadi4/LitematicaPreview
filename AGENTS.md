# Litematica Preview

Windows x64 desktop app. `App/` is Tauri 2 with a Rust host and Fluent UI React;
`Native/` is the Rust Nucleation decoder/mesher library. Seven formats, listed once
in the Rust host, are used by dialogs, drag/drop, and registration.

## Build

- Windows: `./scripts/build.ps1 -Installer` (Node.js, Rust MSVC, C++ Build Tools,
  Windows SDK; WebView2 Evergreen is installed by setup when missing).
- Frontend: `npm --prefix App ci` then `npm --prefix App run build`.
- Rust compile only (after frontend build): `cargo check --manifest-path App/src-tauri/Cargo.toml --all-targets --locked`.
- Decoder checks: `cargo test --manifest-path Native/Cargo.toml --release --locked`.
- Do not report Windows runtime, installer, or benchmark results from a Mac.

## Ownership and constraints

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
