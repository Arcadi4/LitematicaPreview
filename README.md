<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview icon" width="160" />
  <h1>Litematica Preview</h1>
  <p><strong>Fast, offline Minecraft schematic viewer for Windows</strong></p>

  [![CI](https://img.shields.io/github/actions/workflow/status/Arcadi4/LitematicaPreview/ci.yml?style=flat-square&label=CI)](https://github.com/Arcadi4/LitematicaPreview/actions)
  [![Platform](https://img.shields.io/badge/Platform-Windows%20x64-0078d4?style=flat-square&logo=windows)](https://github.com/Arcadi4/LitematicaPreview)
  [![Tauri 2](https://img.shields.io/badge/Tauri-v2-24c8db?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
  [![Fluent UI](https://img.shields.io/badge/Fluent%20UI-React%20v9-0078d4?style=flat-square&logo=react)](https://react.fluentui.dev)
  [![Nucleation](https://img.shields.io/badge/Powered%20by-Nucleation-ff8c00?style=flat-square)](https://github.com/Schem-at/Nucleation)

  [Features](#features) • [Installation](#installation) • [Supported Formats](#supported-formats) • [Controls](#controls) • [Development](#development)
</div>

Litematica Preview is a native Windows desktop application for inspecting and viewing Minecraft schematics and structures in real-time 3D. Adapted from [LitematicaQL](https://github.com/Arcadi4/LitematicaQL) for macOS, it uses [Nucleation](https://github.com/Schem-at/Nucleation) and application-owned bounded readers to decode builds locally, with schematic-mesher generating their geometry. Files are never uploaded to external services.

The app pairs a Rust host with Fluent UI React v9 and an on-demand WebGL 2 renderer, delivering a responsive desktop experience with native Windows shell integration.

## Features

- **Offline & Private**: Decoding and meshing run entirely locally; schematic data is never uploaded.
- **Seven Format Support**: Inspect `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn` files.
- **Accurate Material Meshing**: Opaque, cutout, and transparent geometry are rendered in dedicated passes with repeated greedy meshing and high-fidelity textures.
- **Tight Geometry Framing**: Initial camera framing and **Fit** calculate tight bounding geometry rather than empty chunk boundaries.
- **On-Demand Rendering**: The WebGL 2 engine redraws only on user input, resize, or file loads instead of running a perpetual animation loop.
- **Bounded Transfers**: Spatial chunks are generated with complete neighbor context, sent in acknowledged frames, and uploaded to WebGL through binary reads of at most 1 MiB. Normals and colors use normalized 8-bit attributes.
- **Windows Integration**: Per-user file association management, Fluent UI v9 controls, light/dark/system themes, and drag-and-drop support.

File-open failures and recoverable internal errors display their diagnostic in an
error dialog and return to the home screen. Decoding runs in an isolated worker:
even a native decoder crash does not close the viewer, and the next open starts
a fresh worker. On Windows the worker has a 2 GiB memory limit. Extremely detailed
schematics can exceed this limit and cannot be previewed. The resource pack is
cached while the worker remains healthy.

Native owns Litematic and Sponge readers that fill each region's final block array
directly. It retains compact palette-indexed chunk data for neighbor queries while
generating and releasing one mesh chunk at a time. The host stores one segmented
upload payload, then releases it after upload or cancellation. The WebView uploads
each bounded segment directly to GPU buffers without assembling a complete model
ArrayBuffer. The 2 GiB limit applies to the decoder, not the combined host,
WebView2 and GPU memory.

## Installation

### Requirements

- Windows 10 or 11 (x64)
- A graphics driver with WebGL 2 support
- [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)

> [!NOTE]
> WebView2 is pre-installed on Windows 11 and recent Windows 10 builds. Neither .NET nor Node.js is required at runtime. Once WebView2 is present, previewing works completely offline.

### Options

- **Installer (`LitematicaPreview-<version>-win-x64-setup.exe`)**: Download the setup executable from [Releases](https://github.com/Arcadi4/LitematicaPreview/releases). Installs per-user without requiring administrator privileges. If WebView2 is missing, setup automatically downloads and installs Microsoft's bootstrapper. After Welcome, the file-associations page lets you choose individual extensions; registration and all supported extensions are checked by default.
- **Portable Package (`LitematicaPreview-<version>-win-x64-portable.zip`)**: Download and extract the portable archive and run `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` alongside the executable. Extracting or running the portable package does not register file associations.

## Usage

Open a schematic in any of the following ways:

- Click **Open** (or press `Ctrl+O`) to choose a file.
- Drag and drop a supported file directly into the window.
- Select any of the bundled sample builds on the welcome screen.
- Launch from the command line:

```powershell
.\LitematicaPreview.exe "C:\Schematics\Cottage.litematic"
```

### File Associations

The application can register as a Windows handler for supported schematic formats. Choosing the default handler remains under Windows' control:

- **During setup**: The file-associations page follows Welcome. Registration and all supported extensions start checked; uncheck registration to continue without it, or select only the extensions you want. Registration requires at least one selected extension. The separate checkbox to open Windows Default Apps settings starts unchecked. Silent setup never registers formats or opens Settings. Reinstalling replaces this copy's registered selection; leaving registration disabled removes registrations owned by this copy.
- **From the app**: Open the menu and select **Set as default app…** to register and open Windows Default Apps settings. Select **Remove file associations** to unregister.
- **From the command line** (convenient for portable copies):

```powershell
# Register all supported file associations under HKCU
.\LitematicaPreview.exe --register

# Replace this copy's registered selection with these extensions
.\LitematicaPreview.exe --register-extensions ".litematic,.schem"

# Open Windows Default Apps settings without changing registration
.\LitematicaPreview.exe --default-apps

# Remove file associations registered by this copy
.\LitematicaPreview.exe --unregister
```

> [!IMPORTANT]
> Registrations are written strictly to the current user's registry hive (`HKCU`). The app never overwrites Windows `UserChoice`; choose defaults in Windows Settings or the **Open with** dialog. Removing registrations only removes entries owned by this executable, preserving another installed or portable copy's registrations.

## Supported Formats

| Extension | Format | Description |
| --- | --- | --- |
| `.litematic` | Litematica | Fabric / Litematica mod schematic |
| `.schem` | Sponge Schematic | Sponge schematic v2 and v3 (WorldEdit modern) |
| `.schematic` | MCEdit Schematic | Classic legacy schematic format (Minecraft 1.12 and earlier) |
| `.nbt` | Java Structure Block | Vanilla Java Edition structure NBT |
| `.snbt` | Structure SNBT | Text-based SNBT structure (brace and bracket block states) |
| `.mcstructure` | Bedrock Structure | Minecraft Bedrock Edition structure export |
| `.nusn` | Nucleation Snapshot | Nucleation native binary snapshot |

## Controls

| Action | Input |
| --- | --- |
| **Open schematic** | Click **Open** or press `Ctrl+O` |
| **Orbit camera** | Left-click and drag, or Arrow keys (with preview focused) |
| **Pan camera** | Right-click / Middle-click and drag, or `Shift` + Arrow keys |
| **Zoom** | Mouse scroll wheel, `+` / `-`, or Zoom buttons |
| **Fit geometry** | Click **Fit**, or press `F` / `Home` |
| **Toggle ground grid** | Click **Ground grid** in the toolbar |
| **Return to home** | Click **Home** in the toolbar |
| **Cancel load** | Click **Cancel** or press `Esc` |

> [!TIP]
> The top-right menu provides quick access to **Controls and shortcuts**, appearance settings (System / Light / Dark), and license notices.

## Development

### Prerequisites

- [Node.js 24 LTS](https://nodejs.org/) and pnpm 12.5.1
- Current stable [Rust](https://www.rust-lang.org/) toolchain with the `x86_64-pc-windows-msvc` target installed
- Visual Studio 2022 Build Tools with **Desktop development with C++**, x64 MSVC tools, and Windows SDK
- Microsoft Edge WebView2 Evergreen Runtime

Nucleation `0.10.14` and schematic-mesher `0.2.0` are pinned to their unmodified
crates.io releases. No dependency patch or Vendor directory is required.
Application-owned bounded readers, compact chunk scheduling, neighbor context,
dynamic atlas discovery and mesh ownership adapters live in `Native/src/` and
use the dependencies' public APIs. A first build needs registry access or a
populated Cargo cache; offline viewing does not require a network connection.

### Build from Source

Run the PowerShell build script from the repository root:

```powershell
# Add the MSVC Rust target if not already installed
rustup target add x86_64-pc-windows-msvc

# Build the setup executable and portable ZIP; retain artifacts/win-x64/
./scripts/build.ps1

# Automated build: suppress the optional final folder-opening prompt
./scripts/build.ps1 -NonInteractive
```

Every build creates the following outputs using the version in `App/src-tauri/tauri.conf.json`:

- `artifacts/LitematicaPreview-<version>-win-x64-setup.exe`
- `artifacts/LitematicaPreview-<version>-win-x64-portable.zip`, containing `LitematicaPreview.exe`, `Assets`, `Demos`, and `Licenses` at the archive root
- `artifacts/win-x64/`, the complete portable staging directory, ready to run
- `artifacts/logs/build-<timestamp>.log`, the full build transcript, including compiler and bundler output

The script prints numbered stages, elapsed time, absolute package paths, file sizes, and installation guidance. Failures identify the stage and log path. An interactive terminal may offer to open the artifacts folder when the build completes; CI, redirected input/output, and `-NonInteractive` runs never prompt. Existing `-Installer` commands remain supported and produce the same two packages. GitHub workflows upload both packages and the build logs; releases also include SHA-256 checksums.

### Development Commands

Run the full desktop app with hot-reloading:

```powershell
pnpm --dir App install --frozen-lockfile
pnpm --dir App run tauri dev
```

Run frontend-only development:

```powershell
pnpm --dir App run dev
pnpm --dir App run build
```

Run test suites and code validation:

```powershell
# Native decoder and mesher tests
cargo test --manifest-path Native/Cargo.toml --release --locked

# Host compile and unit tests
pnpm --dir App run build
cargo check --manifest-path App/src-tauri/Cargo.toml --all-targets --locked
cargo test --manifest-path App/src-tauri/Cargo.toml --release --locked

# Code formatting checks
cargo fmt --manifest-path Native/Cargo.toml -- --check
cargo fmt --manifest-path App/src-tauri/Cargo.toml -- --check
```

## Acknowledgements

- [LitematicaQL](https://github.com/Arcadi4/LitematicaQL): The macOS Quick Look previewer this desktop application is adapted from.
- [Nucleation](https://github.com/Schem-at/Nucleation) by [@Nano112](https://github.com/Nano112): Powers the schematic decoding and meshing pipeline.
- [Tauri](https://tauri.app/): Desktop application framework.
- [Fluent UI React](https://react.fluentui.dev/): Windows Fluent Design system components.
