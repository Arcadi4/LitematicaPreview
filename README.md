<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview icon" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>Offline Minecraft schematic viewer for Windows</strong></p>

[![Platform](https://img.shields.io/badge/Platform-Windows%20x64-0078d4?style=flat-square&logo=windows)](https://github.com/Arcadi4/LitematicaPreview)
[![Tauri 2](https://img.shields.io/badge/Tauri-v2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Fluent UI](https://img.shields.io/badge/Fluent%20UI-React%20v9-0078d4?style=flat-square&logo=react)](https://react.fluentui.dev)
[![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

</div>

<!-- README-I18N:START -->

**English** | [中文](./README.zh.md)

<!-- README-I18N:END -->

Litematica Preview is a Windows desktop viewer for Minecraft schematics and structures. It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn` files locally in 3D.

If you are on macOS, check out the sister project [LitematicaQL](https://github.com/Arcadi4/LitematicaQL) for the same speed and smooth Quick Look integration!

## Highlights

- Preview seven Minecraft schematic and structure formats offline in textured 3D with native wgpu (Direct3D 12) and Nucleation.
- Import Minecraft Java resource packs, keep them locally, and preview with the selected pack. Missing textures fall back to the bundled vanilla resources.
- Load large `.litematic` files with compact decoding and chunked previews that avoid retaining the full decompressed file.
- Generate and upload geometry concurrently with 32-block chunks by default, configurable chunk sizes and worker counts, progressive rendering, and cancellable loading progress.
- Decode in a reusable isolated worker capped at 2 GiB on Windows. Home and cancellation terminate the worker and release its cached resources; wide unchunked models currently use Core 0.1.4's compact-position precision.
- Choose an installer with optional file associations or a portable ZIP that needs no installation.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases):

- **Installer (`LitematicaPreview-<version>-win-x64-setup.exe`)**: Installs per-user without administrator privileges and registers selected file associations.
- **Portable (`LitematicaPreview-<version>-win-x64-portable.zip`)**: Extract the archive and run `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` next to the executable.

Requires Windows 10 or 11 (x64) and the [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (pre-installed on Windows 11 and current Windows 10 builds; the setup installer downloads it automatically if missing).

> [!NOTE]
> To change file associations after installation, open the top-right menu and select **Set as default app…** or **Remove file associations**. Portable copies can also register or unregister from PowerShell with `.\LitematicaPreview.exe --register` and `.\LitematicaPreview.exe --unregister`.

## Supported Formats

| Extension      | File format                                   |
| -------------- | --------------------------------------------- |
| `.litematic`   | Litematica                                    |
| `.schem`       | Sponge schematic                              |
| `.schematic`   | MCEdit                                        |
| `.nbt`         | Java structure block                          |
| `.snbt`        | Structure SNBT, brace or bracket block states |
| `.mcstructure` | Bedrock structure                             |
| `.nusn`        | Nucleation snapshot                           |

## Development

Decoding, mesh packing, and first-use texture preparation live in
[LitematicaPreview Core](https://github.com/Arcadi4/litematica-preview-core).
The host transfers prepared bytes into the native renderer (wgpu/Direct3D 12) without converting attributes or rebasing indices.

GPU upload admission is bounded by both batch count and a 64 MiB budget for geometry and texture bytes. One larger batch can upload on its own; this is not a total model or VRAM limit. Home releases scene geometry and the depth/MSAA attachments, which are allocated only when visible geometry needs them. Smaller chunks reduce intermediate meshing peaks; larger worker counts trade memory for throughput.

Run the desktop app in development mode:

```bash
pnpm install --frozen-lockfile
pnpm run tauri:dev
```

Run frontend and Rust checks (the same set CI runs):

```bash
pnpm run check
```

To exercise an unpublished core checkout, resolve its lockfile entry first:

```bash
LP_CORE_PATH=../litematica-preview-core pnpm run resolve:host
LP_CORE_PATH=../litematica-preview-core pnpm run test:host
```

Run `pnpm run resolve:host` without the override to restore the published dependency
before committing.

### Build

- Node.js 24 LTS and pnpm
- Rust stable toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio C++ Build Tools (Desktop development with C++, x64 MSVC, Windows SDK)

```bash
git clone https://github.com/Arcadi4/LitematicaPreview.git
cd LitematicaPreview

rustup target add x86_64-pc-windows-msvc
pnpm run build:win
```

Build outputs (`*-setup.exe`, `*-portable.zip`, and the runnable `win-x64/` directory) are written to `artifacts/`.
Cargo output uses the workspace `target/` directory by default; set `CARGO_TARGET_DIR` to use another directory. The release workflow shares this directory and the Windows MSVC target between tests and packaging.
The Cargo package keeps `LitematicaPreview` as `default-run` so Tauri can select the application alongside the `pressure` binary. Run the pressure tool explicitly with `pnpm run pressure:host -- ...`.

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.
