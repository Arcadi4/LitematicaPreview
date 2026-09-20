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
