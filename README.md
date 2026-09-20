# Litematica Preview

A Windows x64 schematic viewer based on
[LitematicaQL](https://github.com/Arcadi4/LitematicaQL), without the macOS Quick Look
integration. The desktop app uses Tauri 2, Fluent UI React v9, and an on-demand
WebGL 2 renderer. Nucleation decodes and meshes schematics locally; files are not
uploaded to a service.

## Install and run

- **Supported system:** Windows 10 or 11, x64, with a graphics driver that supports
  WebGL 2 and the [Microsoft Edge WebView2 Evergreen Runtime](https://developer.microsoft.com/microsoft-edge/webview2/).
- **Installer:** run the `*-setup.exe` release. Installation is per-user and does
  not require administrator rights for the app. If WebView2 is missing, setup
  downloads and runs Microsoft's bootstrapper, so an internet connection is
  required for that prerequisite.
- **Portable:** extract the complete portable package and run
  `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` beside it.
  WebView2 must already be installed; copying the executable alone is insufficient.
- Neither .NET nor Node.js is required to run the packaged app. Once WebView2 is
  installed, previewing works offline.

Open a file using **Open**, drop a file onto the window, choose a bundled demo,
or pass a path on the command line:

```powershell
.\LitematicaPreview.exe "C:\Schematics\My build.litematic"
```

Supported formats are `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`,
`.mcstructure`, and `.nusn`. Opaque, cutout, and transparent materials retain their
separate rendering behavior. The initial camera and **Fit** frame the actual
geometry rather than padded schematic dimensions.

## Controls

| Action | Control |
| --- | --- |
| Open a schematic | **Open** or `Ctrl+O` |
| Orbit | Left-button drag, or arrow keys with the preview focused |
| Pan | Right- or middle-button drag, or `Shift` + arrow keys |
| Zoom | Mouse wheel, `+` / `-`, or the zoom buttons |
| Fit geometry | **Fit**, `F`, or `Home` with the preview focused |
| Show or hide the ground grid | **Ground grid** |
| Return to the welcome screen | **Home** toolbar command |
| Cancel a pending load | **Cancel** or `Esc` |

The application menu also provides **Controls and shortcuts**, system/light/dark
appearance settings, and **About and licenses**. Rendering happens on changes
and input, not on a perpetual animation timer.

The Rust host caches the resource pack and serializes decoding/meshing on a
background worker. New opens cancel queued work and discard stale results;
Nucleation itself is not interruptible. Meshes cross Tauri IPC as binary buffers,
not JSON vertex arrays. GPU uploads yield between bounded chunks, and superseded
CPU/GPU resources are released. The renderer caps device pixel ratio at 2 and
redraws only for load, resize, or camera/grid input.

### File associations

Setup registers the app as an available handler. The application menu's
**Set as default app…** registers it and opens Windows Default Apps settings;
Windows may require you to choose it for each file type. **Remove file
associations** removes registrations owned by this copy.

For portable copies, the same operations are available without opening the UI:

```powershell
.\LitematicaPreview.exe --register
.\LitematicaPreview.exe --unregister
```

These commands only manage registrations; `--register` does not open Settings.
Registration is under the current user's registry hive. Existing extension
defaults and Windows' protected `UserChoice` are preserved. An unclaimed
extension receives an initial default. A second copy cannot take over an
existing copy's registration: unregister the owner first. Uninstalling or
unregistering a different copy leaves the owner's associations intact.

## Build from source

Install these Windows build prerequisites:

- Node.js 24 LTS, including npm.
- A current stable Rust toolchain with `x86_64-pc-windows-msvc` installed.
- Visual Studio 2022 or newer Build Tools with **Desktop development with C++**,
  the x64 MSVC tools, and a Windows SDK.
- WebView2 Evergreen Runtime for running the app during development.

From the repository root in PowerShell:

```powershell
rustup target add x86_64-pc-windows-msvc
./scripts/build.ps1
./scripts/build.ps1 -Installer
```

The script installs the locked npm dependencies with `npm ci`, invokes Tauri's
release build (which builds the frontend), and passes `--locked` to Cargo. The
default output is the complete portable app at `artifacts/win-x64/`.
`-Installer` also builds a per-user NSIS setup and copies it to
`artifacts/*-setup.exe`. Tauri acquires its NSIS tooling; Inno Setup is not needed.
The script uses the Tauri resource map for both package layouts and statically
links the MSVC runtime. The release app does not depend on a native decoder DLL.

Development with the actual desktop host:

```powershell
npm --prefix App ci
npm --prefix App run tauri -- dev
```

Frontend-only development and type checking/building:

```powershell
npm --prefix App run dev
npm --prefix App run build
```

The frontend-only server cannot provide native file dialogs, schematic loading,
or Windows integration; use `tauri dev` for end-to-end interaction.

Rust formatting and decoder checks:

```powershell
cargo fmt --manifest-path Native/Cargo.toml -- --check
cargo fmt --manifest-path App/src-tauri/Cargo.toml -- --check
cargo test --manifest-path Native/Cargo.toml --release --locked
npm --prefix App run build
cargo check --manifest-path App/src-tauri/Cargo.toml --all-targets --locked
cargo test --manifest-path App/src-tauri/Cargo.toml --release --locked
```

## Licenses and assets

The project follows the GNU AGPL v3 license in `LICENSE`. Dependency notices and
license texts are in `ThirdParty/`; packaged copies include them under
`Licenses/`, accessible through **About and licenses** → **Open licenses folder**.
Historical OpenTK/GLFW notices remain in the repository for provenance, but the
current app does not ship those libraries.

`Assets/pack.zip` contains separate third-party Minecraft textures, not AGPL
project artwork. See `NOTICE` for the Mojang attribution and redistribution
limitations. Bundled demos and existing assets are repository inputs; building
does not require a sibling copy of the macOS project.
