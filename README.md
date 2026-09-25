<div align="center">
  <img src="Assets/icon.png" alt="Litematica Preview icon" width="200"/>
  <h1>Litematica Preview</h1>
  <p><strong>Offline Minecraft schematic viewer for Windows</strong></p>
</div>

Litematica Preview is a Windows desktop viewer for Minecraft schematics and structures, adapted from [LitematicaQL](https://github.com/Arcadi4/LitematicaQL). It previews `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn` files locally in 3D.

## Install

Download the latest release from the [release page](https://github.com/Arcadi4/LitematicaPreview/releases):

- **Installer (`LitematicaPreview-<version>-win-x64-setup.exe`)**: Installs per-user without administrator privileges and registers selected file associations.
- **Portable (`LitematicaPreview-<version>-win-x64-portable.zip`)**: Extract the archive and run `LitematicaPreview.exe`. Keep `Assets`, `Demos`, and `Licenses` next to the executable.

Requires Windows 10 or 11 (x64) and the [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (pre-installed on Windows 11 and current Windows 10 builds; the setup installer downloads it automatically if missing).

> [!NOTE]
> To change file associations after installation, open the top-right menu and select **Set as default app…** or **Remove file associations**. Portable copies can also register or unregister from PowerShell with `.\LitematicaPreview.exe --register` and `.\LitematicaPreview.exe --unregister`.

## Usage & Controls

Open a schematic by double-clicking an associated file in Explorer, dragging a file into the window, pressing `Ctrl+O`, or passing a file path on the command line:

```powershell
.\LitematicaPreview.exe "C:\Schematics\Cottage.litematic"
```

| Action | Input |
| --- | --- |
| Orbit camera | Left-click + drag, or Arrow keys |
| Pan camera | Right-click / Middle-click + drag, or `Shift` + Arrow keys |
| Zoom | Scroll wheel, `+` / `-`, or toolbar buttons |
| Fit to screen | Click **Fit**, or press `F` / `Home` |
| Toggle ground grid | Click **Ground grid** in the toolbar |
| Cancel load | Click **Cancel** or press `Esc` |

For large builds, open **Preview settings** from the top-right menu to adjust chunk separation and the decoder memory limit. The limit is off by default; enabling it starts at 2048 MB and accepts integer values through 8192 MB. Changes apply on the next file open.

**Enable multithreading** is off by default. It requires chunk separation and at least two available logical processors; while enabled, chunk separation cannot be disabled. **Worker threads** starts at 2 and accepts integers up to the smaller of 8 and the available logical processor count. Settings are saved and apply on the next file open. Gzip/NBT parsing stays sequential; block-state expansion or dense-to-compact preparation and chunk meshing use ordered, bounded native worker tasks. Upload pages are prefetched and validated in Web Workers, while WebGL submission stays on the main thread. Memory-first scheduling can use fewer workers than requested: decode batches contain at most 16,384 blocks, mesh concurrency is reduced for large neighbor contexts, and upload preparation holds at most four 1 MiB pages. Worker stacks and mesh outputs can still raise peak memory; the optional decoder limit does not cover host or WebView2 memory. Cancellation stops new tasks and discards queued results, but already-running mesher calls must finish.

**Speed first (no decoder memory limit)** is an additional multithreading option, off by default. It bypasses memory-first mesh concurrency reduction, refills consumed native task slots immediately, and allows the selected number of queued host batches and upload preparation pages (up to 8 rather than the memory-first upload limit of 4). It also disables the decoder's process-memory cap; the limit controls are unavailable while speed-first is active. Turning speed-first or multithreading off restores the saved memory-limit preference on the next load. Work remains ordered and bounded by worker count, and each IPC page is still at most 1 MiB; this is not an unlimited producer queue. Large schematics can nevertheless exhaust system memory or crash the decoder, and more concurrency does not guarantee a faster load.

With multithreading enabled, upload starts as soon as the decoder produces the first complete geometry batch, while subsequent chunks are still being generated. Initial format parsing, integrity checks, compact indexing and atlas preparation finish before the first batch. In memory-first mode the host admits at most two completed batches including the one being uploaded; speed-first uses the selected worker count instead. Both modes allow one additional batch being received or waiting for queue space. Shared textures are uploaded only once. Acknowledging an uploaded batch releases its host CPU buffers. Batch counts do not impose a fixed byte limit on individual chunks. The renderer stages GPU resources and displays the complete model only after a validated final summary; cancellation or a later decode failure discards the staged model. With multithreading disabled, generation still finishes before upload begins.

Loading shows indeterminate decoding, then progress by processed mesh chunks and uploaded model bytes. In multithreaded mode, generated chunks and cumulative uploaded bytes are shown together; the final upload size is not known until generation finishes. Chunk counts are not time estimates, and 100% generation does not mean upload has finished. During loading, the footer shows one process-memory figure: the sum of the host and decoder's Windows private working sets (resident private pages). WebView2 and GPU memory are excluded, so this is not the entire application's memory use; the value is unavailable if either process cannot be sampled. Model data is the uploaded geometry and texture byte count, not process or GPU memory; it remains visible with the preview and clears on Home.

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
cargo test --manifest-path Native/Cargo.toml --release --locked
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
