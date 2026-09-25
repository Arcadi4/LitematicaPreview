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

For large builds, open **Preview settings** from the top-right menu to adjust chunk separation and the decoder memory limit. The limit is disabled by default; enabling it starts at 2048 MB and accepts integer values through 8192 MB. It remains effective with either multithreaded scheduling mode. It applies only to the isolated decoder process, not the host, WebView2 or GPU. If decoding stops while a cap is enabled, loading returns to Home with an error dialog; a decoder crash alone does not prove that the cap was reached. Changes apply on the next file open.

**Enable multithreading** is enabled by default when at least two logical processors are available. It requires chunk separation; while enabled, chunk separation cannot be disabled. **Worker threads** starts at 4, capped at the smaller of 8 and the available logical processor count. If fewer than two processors are available, multithreading is disabled. Settings are saved and apply on the next file open. Gzip/NBT parsing stays sequential; block-state expansion or dense-to-compact preparation and chunk meshing use ordered, bounded native worker tasks. Upload pages are prefetched and validated in Web Workers, while WebGL submission stays on the main thread. Conservative scheduling can use fewer workers than requested: decode batches contain at most 16,384 blocks, mesh concurrency is reduced for large neighbor contexts, and upload preparation holds at most four 1 MiB pages. Worker stacks and mesh outputs can still raise peak memory; the optional decoder limit does not cover host or WebView2 memory. Cancellation stops new tasks and discards queued results, but already-running mesher calls must finish.

**Conservative memory scheduling** is available with multithreading and disabled by default. Enable it to reduce concurrent mesh work, drain each native worker window before admitting more tasks, and limit queued host batches to two and upload preparation to four pages. This may reduce decoding speed. Leaving it disabled uses the selected worker count without conservative mesh throttling, refills native task slots as results are consumed, and admits up to the selected number of host batches and upload preparation pages (at most 8). Both choices keep work ordered and bounded; neither guarantees a fixed peak memory footprint. The independent decoder process-memory cap remains effective when enabled, and reaching it may stop the decoder and fail the load. Each IPC page remains at most 1 MiB; larger concurrency can still exhaust system memory or crash the decoder.

With multithreading enabled, upload starts as soon as the decoder produces the first complete geometry batch, while subsequent chunks are still being generated. Initial format parsing, integrity checks, compact indexing and atlas preparation finish before the first batch. Conservative scheduling admits at most two completed host batches including the one being uploaded; turning it off uses the selected worker count instead. Both modes allow one additional batch being received or waiting for queue space. Shared textures are uploaded only once. Acknowledging an uploaded batch releases its host CPU buffers. Batch counts do not impose a fixed byte limit on individual chunks. The renderer stages GPU resources and displays the complete model only after a validated final summary; cancellation or a later decode failure discards the staged model. With multithreading disabled, generation still finishes before upload begins.

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
