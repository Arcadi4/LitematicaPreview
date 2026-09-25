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

Litematica Preview is a Windows desktop viewer for Minecraft schematics and structures, adapted from [LitematicaQL](https://github.com/Arcadi4/LitematicaQL). It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn` files locally in 3D.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases):

- **Installer (`LitematicaPreview-<version>-win-x64-setup.exe`)**: Installs per-user without administrator privileges and registers selected file associations.
- **Portable (`LitematicaPreview-<version>-win-x64-portable.zip`)**: Extract the archive and run `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` next to the executable.

Requires Windows 10 or 11 (x64) and the [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (pre-installed on Windows 11 and current Windows 10 builds; the setup installer downloads it automatically if missing).

> [!NOTE]
> To change file associations after installation, open the top-right menu and select **Set as default app…** or **Remove file associations**. Portable copies can also register or unregister from PowerShell with `.\LitematicaPreview.exe --register` and `.\LitematicaPreview.exe --unregister`.

## Supported Formats

| Extension | File format |
| --- | --- |
| `.litematic` | Litematica |
| `.schem` | Sponge schematic |
| `.schematic` | MCEdit |
| `.nbt` | Java structure block |
| `.snbt` | Structure SNBT, brace or bracket block states |
| `.mcstructure` | Bedrock structure |
| `.nusn` | Nucleation snapshot |

## Development

Run the desktop app in development mode:

```powershell
pnpm --prefix App install --frozen-lockfile
pnpm --prefix App exec tauri dev
```

Run frontend and Rust checks:

```powershell
pnpm --prefix App run build
cargo test --manifest-path Mesher/Cargo.toml --release --locked
cargo test --manifest-path App/src-tauri/Cargo.toml --release --locked
```

### Build

- Node.js 24 LTS and pnpm
- Rust stable toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio C++ Build Tools (Desktop development with C++, x64 MSVC, Windows SDK)

```powershell
git clone https://github.com/Arcadi4/LitematicaPreview.git
cd LitematicaPreview

rustup target add x86_64-pc-windows-msvc
./scripts/build.ps1
```

Build outputs (`*-setup.exe`, `*-portable.zip`, and the runnable `win-x64/` directory) are written to `artifacts/`.

## Acknowledgements

Great thanks to [@Nano112](https://github.com/Nano112)'s project [Nucleation](https://github.com/Schem-at/Nucleation) for powering the parsing and meshing pipeline, and to [LitematicaQL](https://github.com/Arcadi4/LitematicaQL) for the original macOS implementation.
