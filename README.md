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

If you are on **macOS**, check out the sister project **[LitematicaQL](https://github.com/Arcadi4/LitematicaQL)** for the same speed and smooth Quick Look integration!

## Highlights

- Preview eight Minecraft schematic and structure formats offline in textured 3D with native wgpu (Direct3D 12 on Windows, Vulkan on Linux) and Nucleation.
- Import Minecraft Java resource packs, keep them locally, and arrange an enabled stack by priority. Missing resources fall back to lower packs, then bundled vanilla.
- Load large `.litematic` files with compact decoding and chunked previews that avoid retaining the full decompressed file.
- Generate and upload geometry concurrently with 32-block chunks by default, configurable chunk sizes and worker counts, progressive rendering, and cancellable loading progress.
- Decode in a reusable isolated worker capped at 2 GiB on Windows. Home and cancellation terminate the worker and release its cached resources; wide unchunked models currently use Core 0.1.4's compact-position precision.
- Choose a Windows installer or portable ZIP, or Linux Debian and AppImage packages.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases).

### Windows x64

Pick the **installer** (`LitematicaPreview-<version>-x86_64-setup.exe`) for a one-click, per-user setup with updates from the About dialog, or the **portable ZIP** (`LitematicaPreview-<version>-x86_64-portable.zip`): extract it, run `LitematicaPreview.exe`, and keep its files together; portable copies also update in place.

Requires Windows 10 or 11 (x64). The installer adds the [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) if missing. After first launch the app asks once to register file types for Open with; it does not change your defaults. Change them later from the top-right menu, or use `.\current\LitematicaPreview.exe --register` / `--unregister` from the extracted portable directory.

### Linux x64

Install the **Debian package** (`LitematicaPreview-<version>-x86_64.deb`) with `sudo apt install ./LitematicaPreview-<version>-x86_64.deb`, or make the **AppImage** (`LitematicaPreview-<version>-x86_64.AppImage`) executable with `chmod +x` and run it.

Releases target Ubuntu 22.04 and need GTK 3, WebKitGTK 4.1, a Vulkan loader and compatible driver, and `xdg-utils`; older distributions are not guaranteed to run the release binaries. Without a Vulkan device the app opens but cannot preview schematics. Debian packages declare these dependencies, install a desktop entry and MIME declarations, and leave your default applications unchanged — choose associations in your file manager.

Debian packages built with the apt repository also enroll a signed source and keyring (removed with the package), so later releases arrive through apt:

```bash
sudo apt update
sudo apt install litematica-preview
```

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.
