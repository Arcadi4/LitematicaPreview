<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview icon" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>Minecraft schematic viewer for Windows and Linux</strong></p>

[![Platform](https://img.shields.io/badge/Platform-Windows%20%26%20Linux%20x64-0078d4?style=flat-square)](https://github.com/Arcadi4/LitematicaPreview)
[![Tauri 2](https://img.shields.io/badge/Tauri-v2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Fluent UI](https://img.shields.io/badge/Fluent%20UI-React%20v9-0078d4?style=flat-square&logo=react)](https://react.fluentui.dev)
[![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

</div>

<!-- README-I18N:START -->

**English** | [中文](./README.zh.md)

<!-- README-I18N:END -->

Litematica Preview is a Windows and Linux desktop viewer for Minecraft schematics and structures. It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.mca`, and `.nusn` files locally in 3D.

If you are on macOS, check out the sister project [LitematicaQL](https://github.com/Arcadi4/LitematicaQL) for the same speed and smooth Quick Look integration!

## Highlights

- Preview eight Minecraft schematic and structure formats offline in textured 3D with native wgpu (Direct3D 12 on Windows, Vulkan on Linux) and Nucleation.
- Import Minecraft Java resource packs, keep them locally, and arrange an enabled stack by priority. Missing resources fall back to lower packs, then bundled vanilla.
- Load large `.litematic` files with compact decoding and chunked previews that avoid retaining the full decompressed file.
- Generate and upload geometry concurrently with 32-block chunks by default, configurable chunk sizes and worker counts, progressive rendering, and cancellable loading progress.
- Decode in a reusable isolated worker capped at 2 GiB on Windows. Home and cancellation terminate the worker and release its cached resources; wide unchunked models currently use Core 0.1.4's compact-position precision.
- Choose a Windows installer or portable ZIP, or Linux Debian and AppImage packages.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases):

### Windows x64

- **Installer (`LitematicaPreview-<version>-win-x64-setup.exe`)**: Installs per-user without administrator privileges and registers selected file associations.
- **Portable (`LitematicaPreview-<version>-win-x64-portable.zip`)**: Extract the archive and run `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` next to the executable.

Requires Windows 10 or 11 (x64) and the [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (pre-installed on Windows 11 and current Windows 10 builds; the setup installer downloads it automatically if missing).

> [!NOTE]
> To change file associations after installation, open the top-right menu and select **Set as default app…** or **Remove file associations**. Portable copies can also register or unregister from PowerShell with `.\LitematicaPreview.exe --register` and `.\LitematicaPreview.exe --unregister`.

### Linux x64

- **Debian package (`LitematicaPreview-<version>-linux-x64.deb`)**: Install with `sudo apt install ./LitematicaPreview-<version>-linux-x64.deb`.
- **AppImage (`LitematicaPreview-<version>-linux-x64.AppImage`)**: Make it executable with `chmod +x`, then run it.

Linux releases target Ubuntu 24.04 and require GTK 3, WebKitGTK 4.1, a Vulkan loader and compatible driver, and `xdg-utils`. The Debian package declares these dependencies. Older distributions are not guaranteed to run binaries built on the release runner.

The Debian package installs a desktop entry and MIME declarations without changing your default applications. Choose file associations through your desktop's file manager; the Windows registration commands are not shown on Linux.

## Texture Packs

Open **Settings → Texture packs** to import Minecraft Java resource-pack ZIPs. Enable packs with the switches and arrange them with the arrow buttons, highest priority first. New imports start at the lowest custom priority.

Missing textures, models, and blockstates fall back through lower packs to bundled vanilla. Changes reload the current preview. Saved packs and the previous single-pack selection are preserved; removing a saved copy never deletes the original ZIP.

## Supported Formats

| Extension      | File format                                   |
| -------------- | --------------------------------------------- |
| `.litematic`   | Litematica                                    |
| `.schem`       | Sponge schematic                              |
| `.schematic`   | MCEdit                                        |
| `.nbt`         | Java structure block                          |
| `.snbt`        | Structure SNBT, brace or bracket block states |
| `.mcstructure` | Bedrock structure                             |
| `.mca`         | Minecraft Anvil region                        |
| `.nusn`        | Nucleation snapshot                           |

## Development

Decoding, mesh packing, and first-use texture preparation live in
[LitematicaPreview Core](https://github.com/Arcadi4/litematica-preview-core).
The host transfers prepared bytes into the native wgpu renderer without converting attributes or rebasing indices. Windows presents directly through a Direct3D 12 swapchain beneath WebView2. Linux renders Vulkan frames offscreen and hands their pixels to a GTK DrawingArea beneath transparent WebKitGTK. This keeps GTK in charge of X11/Wayland composition and costs one GPU readback per rendered frame; geometry never crosses the WebView bridge.

GPU upload admission is bounded by both batch count and a 64 MiB budget for geometry and texture bytes. One larger batch can upload on its own; this is not a total model or VRAM limit. Home releases scene geometry and the depth/MSAA attachments, which are allocated only when visible geometry needs them. Smaller chunks reduce intermediate meshing peaks; larger worker counts trade memory for throughput.

Linux presentation uses a bounded latest-frame mailbox and pooled pixel buffers. Readback waits run on a dedicated worker, while GTK objects stay on the main thread. Resize drops mismatched frames instead of stretching them, and idle windows do not schedule a rendering timer.

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

### Build Windows

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

### Build Linux

Use a Linux x64 host with Node.js 24, pnpm 12, and Rust stable. On Ubuntu 24.04, install the native build dependencies:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev libssl-dev librsvg2-dev \
  libayatana-appindicator3-dev libvulkan-dev mesa-vulkan-drivers patchelf
```

```bash
pnpm install --frozen-lockfile
rustup target add x86_64-unknown-linux-gnu
pnpm run build:linux
```

The Tauri build prepares the frontend and bundles resources. Versioned `.deb` and `.AppImage` files are written to `artifacts/`; `CARGO_TARGET_DIR` overrides the Cargo output directory. Run `pnpm run checksums --linux` for Linux packages, or pass both `--win --linux` for a combined release.

Before treating a Linux release as stable, verify live composition on both X11 and Wayland: first load, Home/cancellation, resize/maximize/fullscreen, HiDPI, pointer and keyboard input, native dialog modality, minimize/restore, and translucent overlays during prolonged rendering. Container or offscreen checks do not establish physical-desktop or driver compatibility.

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.
