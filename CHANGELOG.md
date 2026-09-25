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

- Supports `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, and `.nusn` formats
- Fast offline rendering powered by WebGL 2 and Nucleation
- Distributed as both an installer (`.exe`) and a portable zip package

## ToDos

- Decrease memory usage when decoding schematic and structure
- Make memory limit a changeable option
- Support "history files" feature
- Add lightning shader
