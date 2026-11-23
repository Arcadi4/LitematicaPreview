<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview icon" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>Minecraft schematic viewer for Windows, macOS, and Linux</strong></p>

[![Rune](https://img.shields.io/badge/UI-Rune-5546eb?style=flat-square)](https://github.com/anomalyco/rune)
[![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

</div>

<!-- README-I18N:START -->

**English** | [中文](./README.zh.md)

<!-- README-I18N:END -->

Litematica Preview is a native desktop viewer for Windows, macOS, and Linux. It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.mca`, and `.nusn` files locally in 3D.

## Highlights

- Open several schematics in one window with tabs. Open a folder or a single file to browse nearby schematics with cached thumbnails.
- Orbit, pan, zoom, or fly through the preview. Set key bindings for both modes and adjust spectator field of view and mouse sensitivity in Preferences.
- Copy the preview or save it as PNG or JPEG. Stack Minecraft Java resource packs in priority order, with bundled textures as the fallback.
- Preview large schematics progressively with configurable chunk sizes and worker counts.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases).

### Windows x64

Choose the **installer** (`LitematicaPreview-<version>-x86_64-setup.exe`) to select an install directory and optionally register file types. Or extract the **portable ZIP** (`LitematicaPreview-<version>-x86_64-portable.zip`) and run `LitematicaPreview.exe`.

### macOS

Open the `.dmg` for Apple silicon (`aarch64`) or Intel (`x86_64`), then drag Litematica Preview to Applications. You can also install it with `brew install --cask Arcadi4/tap/litematica-preview`. For Finder Quick Look, see [LitematicaQL](https://github.com/Arcadi4/LitematicaQL).

### Linux x64

Install the **Debian package** (`LitematicaPreview-<version>-x86_64.deb`) with `sudo apt install ./LitematicaPreview-<version>-x86_64.deb`, or make the **AppImage** (`LitematicaPreview-<version>-x86_64.AppImage`) executable with `chmod +x` and run it.

Linux requires a compatible Vulkan driver. AppImage updates use the bundled [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate/issues). Debian packages do not update from the app. Stable packages enroll a signed apt source for upgrades:

```bash
sudo apt update
sudo apt install litematica-preview
```

### Update sources

Windows and macOS, including Homebrew installations, can check for updates in Preferences. Choose **Cloudflare** if you are in mainland China, or **GitHub** otherwise. Turn off in-app updates to hide the source setting.

To build from source, see [Build and release](docs/build.md).

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the whole parsing and meshing pipeline.
