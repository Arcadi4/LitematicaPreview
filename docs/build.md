# Build and release

Platform builds, signing, packaging, and release publishing for Litematica Preview.

## Development

Install stable Rust and `just`. The app builds from the workspace lockfile.

```bash
just dev
just build
just check
```

`just` calls `cargo xtask` for repository tasks. Set `LP_CORE_PATH` to use a local checkout of `litematica-preview-core`. Do not commit a lockfile resolved with the override.

## Windows

Use the `x86_64-pc-windows-msvc` toolchain, Visual Studio C++ Build Tools, Windows SDK, and Inno Setup.

```powershell
just package-windows
```

The command builds a per-user installer with an editable destination, a portable ZIP, and the standalone update executable in `artifacts/`. For local packaging tests, pass `--allow-unsigned`. Releases require signing. Configure `LITEMATICA_WINDOWS_SIGN_PARAMS` for SignTool, or configure `LITEMATICA_WINDOWS_AZURE_SIGN_METADATA` with `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and `AZURE_CLIENT_SECRET` for Azure Artifact Signing. For local Artifact Signing, set `LITEMATICA_WINDOWS_AZURE_SIGN_DLIB` to the x64 `Azure.CodeSigning.Dlib.dll` from `Microsoft.ArtifactSigning.Client`.

## macOS

Use Xcode and install both Rust desktop targets:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
just package-macos aarch64
just package-macos x86_64
```

Each architecture produces a `.dmg` in `artifacts/`. The app updater downloads this same disk image, verifies the replacement bundle's Developer ID signature against the installed app, and replaces the entire app. For local packaging tests, pass `--allow-unsigned`. Release Actions require `MACOS_CERTIFICATE_P12` (base64), `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGN_IDENTITY`, `MACOS_NOTARY_APPLE_ID`, `MACOS_NOTARY_TEAM_ID`, and `MACOS_NOTARY_PASSWORD`.

## Linux

Build on Linux x86-64. Install Rust, `just`, `build-essential`, `curl`, `file`, `libwayland-dev`, `libxkbcommon-dev`, `pkg-config`, `gnupg`, `xz-utils`, and `zsync`. The published packages target Ubuntu 22.04 or newer and need a compatible Vulkan driver.

```bash
just package-linux
```

The command creates a `.deb`, an `.AppImage`, and its `.zsync` metadata in `artifacts/`. The Debian binary is built with `managed-install`, which disables in-app updates. The AppImage bundles the pinned upstream [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate/issues) and contains its own update information. `LP_APPIMAGETOOL` can select a local appimagetool for packaging.

### Publish the apt repository

A release build enrolls the signed apt repository in the Debian package with `--apt`:

```bash
LP_APT_BASE_URL=https://apt.example.com \
LP_APT_PUBLIC_KEY_FILE=archive-public-key.asc \
just package-linux --apt
```

`--apt` requires the base URL and public key. The base URL must use `https` and point to the bucket's custom domain.

Publishing needs `reprepro`, GnuPG, the Cloudflare CLI (`cf`), and the released `.deb`:

```bash
just apt-publish --deb artifacts/LitematicaPreview-<version>-x86_64.deb
```

The publisher rebuilds a latest-only `stable`/`main`/`amd64` reprepro distribution, verifies `InRelease` with the keyring embedded in the package, then writes the pool package, the `Packages` indexes and their immutable by-hash copies, `Release` and `Release.gpg`, and `InRelease` last. It retains old pool and by-hash objects for clients using an earlier index.

Environment: `LP_APT_BASE_URL`, `LP_APT_GPG_PRIVATE_KEY` (or `LP_APT_GPG_PRIVATE_KEY_FILE`), optional `LP_APT_GPG_PASSPHRASE` and `LP_APT_GPG_FINGERPRINT`, `LP_APT_R2_BUCKET`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, and optional `LP_APT_R2_JURISDICTION`. `--dry-run` builds, signs, and verifies the repository without uploading.

Configure the custom domain so `dists/stable/InRelease` and the `Packages` indexes are not cached, or bypass the cache for `dists/`. A stale cached index can leave clients requesting objects from a different repository generation.

Rotating the signing key needs a transitional release: apt verifies `InRelease` with the keyring installed by the client's current Debian package, so ship the new key before signing with it.

## Nix

`nix build`, `nix run`, and `nix develop` use the workspace's native app. Nix manages updates.

## Release publication

The release workflow validates the version tag against `app/Cargo.toml`, builds all platforms, and publishes versioned artifacts and `SHA256SUMS.txt` to GitHub and the `apps-download` R2 bucket via `cf`. Stable releases update the Cloudflare manifest last at `https://download.tanh.moe/litematica-preview/v1/latest.json`. Prereleases do not change the stable manifest, apt repository, or Homebrew Cask.

The existing `update.tanh.moe` Worker and `litematica-preview/windows/` objects remain live for Velopack clients. Rune does not use or publish to that endpoint. See [#65](https://github.com/Arcadi4/LitematicaPreview/issues/65) for retirement.

Stable publication also updates `Casks/litematica-preview.rb` in `Arcadi4/homebrew-tap`. Configure `HOMEBREW_TAP_TOKEN` as a fine-grained token with Contents read/write on that tap only.

Before release, verify native launch, document opening, tab changes, resizing, input, and image export on each supported desktop. Exercise both X11 and Wayland on Linux. Cross-compilation and headless images do not verify native window integration.

## Icon artwork

The app packages a subset of [Microsoft Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons/tree/a3de63245b4fa49c30e20d9838593a105affe774), MIT-licensed. The vendored files in `app/icons/` are the Regular 20-point SVG bodies with theme-controlled `currentColor` fills for Rune Kit.
