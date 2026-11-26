# Litematica Preview

Windows, macOS, and Linux desktop app for offline Minecraft schematic viewing.

- **Architecture**: `app/` uses Rune Kit for the UI and Rune for the native window and GPU device. The preview compositor draws into a texture on the same device and samples it at the Rune viewport slot. Only export, clipboard, and thumbnails read pixels back.
- **Shared Core**: `litematica-preview-core` owns geometry decoding and meshing. Use `LP_CORE_PATH` with the `just` developer recipes to test a local Core checkout. Resolve with the crates.io dependency before committing `Cargo.lock`.
- **Formats**: Source of truth is `app/src/formats.rs`. Vanilla textures and demos are embedded from `assets/pack.zip` and `fixtures/demos/`.

## Rune

[Rune](https://github.com/anomalyco/rune) is a native GPU-rendered UI framework. Rune apps can be ran headless, therefore you don't need computer use for debugging.

- [Docs](https://github.com/anomalyco/rune/blob/dev/crates/rune/README.md)
- [Components](https://github.com/anomalyco/rune/blob/dev/crates/rune-kit/README.md)
- [Examples](https://github.com/anomalyco/rune/tree/dev/examples)

## Commands

| Task        | Command      |
| ----------- | ------------ |
| Run the app | `just dev`   |
| Build       | `just build` |
| Tests       | `just test`  |
| Local gate  | `just check` |

Platform build, signing, packaging, and apt publishing steps: [`docs/build.md`](./docs/build.md).

## Design

The app uses Fluent UI's icons. We vendor them in the repo since JavaScript is not involved. Feel free to pull new icons if your work requires them.

## Comments

Code explains itself: a reader learns what a function does from its name, parameters, and types, not from prose above the call site. The default is no comment.

- Never restate the code, narrate a body, or repeat a callee's documentation at the call site. Rename, extract, or move the check instead.
- One home per fact: a rationale shared by a type and its methods, or by peers delegating to one helper, lives at that common owner; peers and call sites link to it (`[`Item`]` in Rust, `{@link item}` in TS) instead of re-deriving it.
- Doc comments describe the item's contract — preconditions, postconditions, edge cases, why it exists — never its body.
- Never record replaced implementations, refactor history, or removed files; that belongs in the commit message. A reference to code that no longer exists is a bug in the comment.
- Write what cannot go stale: name a list instead of counting it, state a limit or default once on the item that owns it, link a section by anchor or item rather than quoting it, and keep example names fake.
- Concise and structured: a list or a labelled line beats a paragraph, one word per concept, and values are spelled the way the code spells them.
- A comment earns its place for a hidden invariant, a workaround for a known bug, or a deliberate exception to the surrounding pattern — then it is one line, so the next reader does not take it for an oversight. A behavioral claim is a claim: verify it or leave it out.
