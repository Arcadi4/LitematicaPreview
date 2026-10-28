# v0.4.0

## Features

- Copy the current preview to the clipboard or save it as PNG or JPEG from the toolbar ([#31](https://github.com/Arcadi4/LitematicaPreview/issues/31)).
- Render previews with a native wgpu (Direct3D 12) pipeline that draws directly into the window surface beneath the Fluent interface. Pointer and keyboard gestures are forwarded to the renderer, and load progress arrives as events instead of streamed geometry ([#17](https://github.com/Arcadi4/LitematicaPreview/issues/17)).
- Import Minecraft Java resource packs, keep them in the app data folder, and arrange the enabled packs by priority. Missing textures, models, and block states fall back through lower packs to bundled vanilla, and the previous single-pack selection is migrated.
- Preview Minecraft Anvil region files (`.mca`).

## Improvements

- Feed the native renderer from bounded in-process mesh sessions. Prepared batches reach the GPU without converting attributes or rebasing indices, upload admission is bounded by batch count and a 64 MiB staging budget, and upload credits track the actual worker count.
- Discard transient render attachment contents and release the depth and MSAA attachments when no visible geometry needs them.
- Share material uniforms within geometry batches, reducing per-part buffer allocations.
- Reduce default preview chunk memory peaks and parallelize palette analysis.
- Add a host pressure suite with pinned decode baselines and frame metrics on Windows hardware, with pull-request reports that fail on regressions.
- Port the test suites to TypeScript and add the comment policy to the contributor guide.

## Fixes

- Bound viewport backing sizes to device limits and negotiate native adapter buffer capabilities instead of assuming them.
- Preserve durable viewport state across scene resets and camera depth when the final geometry bounds arrive.
- Fit and clip the camera to the visible stage rectangle, and keep pointer mapping aligned with the native viewport across resizes.
- Restore isolated decoder failure boundaries and binary transfer recovery on cancellation.
- Keep the viewport clear color in the linear color space.
- Set `default-run` to `LitematicaPreview` so `cargo run` starts the app.

## ToDos

- Add recent-files history.
- Add lighting shader.
- Multi-file preview ([#13](https://github.com/Arcadi4/LitematicaPreview/issues/13))

# v0.3.1

## Features

- Add Linux x64 Debian/AppImage packaging and a Vulkan offscreen viewport composited beneath WebKitGTK by GTK, with bounded asynchronous frame readback and parented modal file pickers ([#28](https://github.com/Arcadi4/LitematicaPreview/issues/28)). Windows retains its Direct3D 12 surface; macOS support is unchanged.
- Extend saved texture packs with enabled priority stacks, fallback through lower packs to vanilla, and migration of the previous selection ([#29](https://github.com/Arcadi4/LitematicaPreview/issues/29)).
- Gather preview settings, control help, and theme choice in a single settings dialog with Performance, Control, and Appearance sections in a scrollable sidebar layout.
- Make every preview setting more compact and add an info notice that explains what the setting does without leaving the dialog.

## Improvements

- Keep the window responsive while uploading models by yielding the render loop once per upload page instead of once per GPU allocation.

## ToDos

- Add recent-files history.
- Add lighting shader.
- Multi-file preview ([#13](https://github.com/Arcadi4/LitematicaPreview/issues/13))

# v0.3.0

## Features

- Enable multithreaded preview generation by default with 4 worker threads when enough logical processors are available. Existing saved settings are preserved. Loading time reduced by about 50%.
- Add a conservative memory scheduling option for multithreaded previews. It is disabled by default; enabling it reduces concurrent mesh work and queued upload batches, which may slow decoding.
- Show the combined host and decoder private working sets during loading. This figure excludes WebView2 and GPU memory.

## Improvements

- Start uploading complete geometry batches while later chunks are still being generated. Keep mesh work, host batches, and upload preparation bounded, and discard staged results on cancellation or failure.
- Pack preview data into shared arenas for fewer IPC reads and GPU buffer allocations without changing draw order.
- Keep the optional decoder process-memory limit effective with either scheduling mode.

## ToDos

- Add recent-files history.
- Add lighting shader.

# v0.2.0

## Features

- Add preview settings for chunk separation and an optional decoder memory limit. The limit is off by default; when enabled, it starts
  at 2048 MB and accepts values up to 8192 MB.
- Show separate loading progress for decoding, mesh generation, and model upload.
- Display decoder memory while loading and uploaded model-data size in the footer.

## Improvements

- Replace the chunk-size selector with a five-stop slider: 16, 32, 64, 128, and 256 blocks per side.
- Reduce peak decoding memory for .litematic previews with fixed-buffer, two-pass streaming and compact block indexing. Peak memory usage dropped by **90%**.
- Avoid retaining the full decompressed .litematic document and packed block-state arrays during preview decoding.

## ToDos

- Add recent-files history.
- Add lighting shader.
- Continue improving memory use for formats that retain dense decoding paths.

# v0.1.0

> Initial release of Litematica Preview — an offline Minecraft schematic and structure viewer for Windows x64.

## Highlights

- Supports `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.mca`, and `.nusn` formats
- Fast offline rendering powered by WebGL 2 and Nucleation
- Distributed as both an installer (`.exe`) and a portable zip package

## ToDos

- Decrease memory usage when decoding schematic and structure
- Make memory limit a changeable option
- Support "history files" feature
- Add lightning shader
