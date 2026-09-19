# Litematica Preview

Windows x64 desktop app. `App/` is C#/.NET 10 WinForms and OpenTK; `Native/`
is the Rust Nucleation decoder/mesher DLL. Seven formats, listed once in
`NativePreview.Extensions`, are used by dialogs, drag/drop, and registration.

## Build

- Windows: `./scripts/build.ps1 -Installer` (.NET 10, Rust MSVC, C++ Build Tools,
  Windows SDK, Inno Setup 6).
- C# compile only on macOS: `dotnet build App/LitematicaPreview.csproj -c Release`.
- Rust compile only: `cargo check --manifest-path Native/Cargo.toml --all-targets --locked`.
- Windows checks: `cargo test --manifest-path Native/Cargo.toml --release --locked`
  (on Windows). See `docs/windows-validation.md`.
- Do not report Windows runtime, installer, or benchmark results from a Mac.

## Ownership and constraints

- `NativePreview` owns a Rust preview until synchronous GPU uploads finish.
  Borrowed pointers must not survive disposal; keep all GL calls on the UI thread.
- The native resource pack is cached, with one active decode/mesh call per process.
  Cancel queued work and dispose stale results; Nucleation itself is not cancellable.
- Keep opaque, cutout, and transparent parts separate. Greedy materials use their
  own repeating textures. The atlas is shared and clamped. Never flatten them.
- Include greedy parts in triangle counts and bounds. Nucleation 0.10.14's
  `total_triangles()` and `is_empty()` omit them.
- Frame geometry bounds rather than chunk-padded schematic dimensions.
- Render on Paint/resize/input. No perpetual frame timer, animations, web bridge,
  compatibility shell, or new general-purpose service layer.
- Preserve Nucleation's exact version pin, bounded decoding, Java binary NBT
  fallback, and brace-state SNBT normalization until real fixtures justify changes.
- Register under HKCU. Quote executable and filename separately. Never overwrite
  Windows UserChoice or unregister another installed copy's ProgID.
- `Assets/pack.zip`, fixtures, icons, and license texts are tracked inputs.
  No build requires the sibling macOS repository or a Node toolchain.
