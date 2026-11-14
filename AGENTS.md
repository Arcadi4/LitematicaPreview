# Litematica Preview

Windows and Linux desktop app for offline Minecraft schematic and structure viewing; release packages target x64, and macOS is unsupported.

- **Architecture**: Host (`src-tauri/`) renders beneath a transparent WebView overlay (`src/`). Windows presents a Direct3D 12 swapchain under WebView2. Linux reads Vulkan offscreen frames into a GTK DrawingArea under WebKitGTK, with GTK owning native composition on X11 and Wayland. The frontend manages UI state and controls; it renders zero canvas pixels.
- **Shared Core**: Geometry decoding, caching, and meshing live in `litematica-preview-core`, a crates.io dependency pinned by `Cargo.lock`. Every `*:host` script runs through `scripts/cargo.ts`, which swaps in a local checkout when `LP_CORE_PATH` is set.
- **Formats**: `.litematic`, `.schem`, `.schematic`, `.nbt`, `.snbt`, `.mcstructure`, `.mca`, `.nusn`, registered in `src-tauri/src/formats.rs`.
- **Fixtures**: Default pack in `Assets/pack.zip`; format and demo fixtures in `Fixtures/`; million-block pressure fixtures in `Fixtures/Pressure/` with pinned decode counts in its `baselines.json`.

## Commands

| Task           | Command                  |
| -------------- | ------------------------ |
| App dev        | `pnpm run tauri:dev`     |
| Frontend tests | `pnpm run test`          |
| Host tests     | `pnpm run test:host`     |
| Full CI gate   | `pnpm run check`         |
| Pressure run   | `pnpm run pressure:host` |

`pnpm run check` is the gate: format, lint, Rust format check, build, and both test suites. The pressure run needs a hardware adapter, exits nonzero on a breach, and reports under `target/pressure/`.

## Architecture & Operational Constraints

- **Renderer & Geometry**: Core's `MeshStream` owns upload-ready batches; consume them natively without TCP or geometry copies. Its worker-sized mesh window generates as a whole and refills after exhaustion; rolling refill and float-position fallback are suspended Core work. One GPU geometry buffer per batch; slice vertex/index ranges per part.
- **Materials**: Never flatten opaque, cutout, and transparent parts. Greedy materials use dedicated repeating textures; the atlas is shared and clamped. Include greedy parts in triangle counts and bounds.
- **Framing & Uploads**: Start from occupied source bounds, then fit actual geometry bounds at completion only if the user has not manipulated the camera. Present the first visible batch immediately and coalesce later arrivals; resize, input, and completion render immediately, with no perpetual timer. GPU upload credits equal actual workers: acquire before GPU allocation, submit per batch, release only on GPU completion, and drain before reporting success. Wait outside renderer locks.
- **Loading Session**: One in-process background session caches resources and serializes decode/mesh work; pending requests collapse to the latest. New loads, cancellation, failure, and Home clear displayed GPU geometry. Catch unwindable native panics and ordinary errors; aborts, access violations, and some OOM failures may terminate the app. Memory limits and conservative scheduling are suspended, not ineffective switches. Host memory telemetry excludes WebView2 and GPU memory.
- **Registry & Windows Integration**: Register file associations under `HKCU`. Quote executable and file paths separately. Never overwrite Windows `UserChoice` or unregister another installed app's ProgID.
- **Windows Updates**: Velopack hooks run before Tauri startup (host CLI modes excepted) and own install, update, and uninstall. In-app updates appear only when `UpdateManager` locates a Velopack installation. Packages must be code-signed before publishing, and the one-click setup has no association page, so the app asks once after first launch.
- **Linux Presentation**: `tauri.linux.conf.json` must create the main window with `transparent: true` before realization. Size offscreen frames from the GTK DrawingArea allocation and scale, not the top-level window (which may include client-side decorations and shadows). Keep WebKit input on the overlay and GTK/Cairo objects on the main thread. Offscreen readback uses one worker and latest-request/frame mailboxes, drops obsolete work, reuses staging/pixel allocations, and performs no idle polling or pixel encoding. Only pixels are copied; geometry remains native. Bundled desktop/MIME entries must not override default applications.
- **Linux Verification**: Live composition, startup, resize and fullscreen, HiDPI, input, dialogs, cancellation, and prolonged translucency require a native X11 or Wayland desktop; container and software/offscreen runs are not proof.
- **Host Verification**: Direct3D 12 compositing, transparent WebView2 layering, pointer mapping, framing, multi-monitor DPI scaling, streaming cancellation, and device loss require Windows hardware; never report Windows runtime verification from another platform. A viewport change must also show no startup flash, no stretch or ghosting after resize, snap, maximize, or fullscreen, working canvas focus for Tab and arrow keys, modal dialogs above the main window, suspended presenting under minimize, occlusion, and alt-tab, and intact compositing under long-run translucent DOM overlays.
- Never add JavaScript files (including `.mjs`, `.cjs`). Always use TypeScript.

## Comments

Code explains itself: a reader learns what a function does from its name, parameters, and types, not from prose above the call site. The default is no comment.

- Never restate the code, narrate a body, or repeat a callee's documentation at the call site. Rename, extract, or move the check instead.
- One home per fact: a rationale shared by a type and its methods, or by peers delegating to one helper, lives at that common owner; peers and call sites link to it (`[`Item`]` in Rust, `{@link item}` in TS) instead of re-deriving it.
- Doc comments describe the item's contract — preconditions, postconditions, edge cases, why it exists — never its body.
- Never record replaced implementations, refactor history, or removed files; that belongs in the commit message. A reference to code that no longer exists is a bug in the comment.
- Write what cannot go stale: name a list instead of counting it, state a limit or default once on the item that owns it, link a section by anchor or item rather than quoting it, and keep example names fake.
- Concise and structured: a list or a labelled line beats a paragraph, one word per concept, and values are spelled the way the code spells them.
- A comment earns its place for a hidden invariant, a workaround for a known bug, or a deliberate exception to the surrounding pattern — then it is one line, so the next reader does not take it for an oversight. A behavioral claim is a claim: verify it or leave it out.
