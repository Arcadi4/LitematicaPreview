# Build and release

Platform builds, signing, packaging, and release publishing for Litematica Preview.

## Windows

- Node.js 24 LTS and pnpm
- Rust stable toolchain (`x86_64-pc-windows-msvc`)
- Visual Studio C++ Build Tools (Desktop development with C++, x64 MSVC, Windows SDK)
- .NET 10 SDK with Velopack: `dotnet tool install --global vpk --version 1.2.161`

```bash
git clone https://github.com/Arcadi4/LitematicaPreview.git
cd LitematicaPreview

rustup target add x86_64-pc-windows-msvc
pnpm run build:win
```

Set `LITEMATICA_WINDOWS_SIGN_PARAMS` for `signtool` arguments or `LITEMATICA_WINDOWS_AZURE_SIGN_FILE` for Azure Artifact Signing metadata. Local unsigned test builds only: `pnpm run build:win -- --allow-unsigned`.

CI accepts `LITEMATICA_WINDOWS_SIGN_PARAMS`, or `LITEMATICA_WINDOWS_AZURE_SIGN_METADATA` with `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and `AZURE_CLIENT_SECRET` as GitHub Actions secrets. Supply the signing provider's certificate or Azure Artifact Signing configuration before a release.

Build outputs are written to `artifacts/`: `*-setup.exe`, `*-portable.zip`, and a `velopack/` directory holding `releases.win.json` plus the full (and optional delta) `.nupkg` files for `https://update.tanh.moe/litematica-preview/windows/`. The runnable staging directory remains `x86_64/`.
Cargo output uses the workspace `target/` directory by default; set `CARGO_TARGET_DIR` to use another directory. The release workflow shares this directory and the Windows MSVC target between tests and packaging.
The Cargo package keeps `LitematicaPreview` as `default-run` so Tauri can select the application alongside the `pressure` binary.

## Linux

Use a Linux x64 host with Node.js 24, pnpm 12, and Rust stable. On Ubuntu 22.04, install the native build dependencies:

```bash
sudo apt-get update
sudo apt-get install --yes build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libxdo-dev libssl-dev librsvg2-dev \
  libayatana-appindicator3-dev libvulkan-dev mesa-vulkan-drivers patchelf
```

```bash
pnpm install --frozen-lockfile
rustup target add x86_64-unknown-linux-gnu
pnpm run build:linux
```

The Tauri build prepares the frontend and bundles resources. Versioned `.deb` and `.AppImage` files are written to `artifacts/`; `CARGO_TARGET_DIR` overrides the Cargo output directory. Run `pnpm run checksums --linux` for Linux packages, or pass both `--win --linux` for a combined release.

### Publish the apt repository

A release build enrolls the signed apt repository in the Debian package with `--apt`:

```bash
LP_APT_BASE_URL=https://apt.example.com \
LP_APT_PUBLIC_KEY_FILE=archive-public-key.asc \
pnpm run build:linux:apt
```

`build:linux:apt` fails when the base URL or a usable public key is missing, and it verifies that the generated `.deb` contains the rendered source and keyring bytes. The base URL must use `https`; it is the public custom domain that serves the bucket.

Publishing needs `reprepro`, GnuPG, the Cloudflare CLI (`cf`), and the released `.deb`:

```bash
pnpm run apt:publish -- --deb artifacts/LitematicaPreview-<version>-x86_64.deb
```

The publisher rebuilds a latest-only `stable`/`main`/`amd64` reprepro distribution, verifies `InRelease` with the keyring embedded in the package, then writes the pool package, the `Packages` indexes and their immutable by-hash copies, `Release` and `Release.gpg`, and `InRelease` last. It retains old pool and by-hash objects for clients using an earlier index.

Environment: `LP_APT_BASE_URL`, `LP_APT_GPG_PRIVATE_KEY` (or `LP_APT_GPG_PRIVATE_KEY_FILE`), optional `LP_APT_GPG_PASSPHRASE` and `LP_APT_GPG_FINGERPRINT`, `LP_APT_R2_BUCKET`, `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, and optional `LP_APT_R2_JURISDICTION`. `--dry-run` builds, signs, and verifies the repository without uploading. Publication stops at the uploads: the public custom domain sits behind the zone's WAF and bot products, which can refuse the publisher while serving clients the same object, so a fetch-back does not gate the release.

Configure the custom domain so `dists/stable/InRelease` and the `Packages` indexes are not cached, or bypass the cache for `dists/`. A stale cached index can leave clients requesting objects from a different repository generation.

Rotating the signing key needs a transitional release: apt verifies `InRelease` with the keyring installed by the client's current Debian package, so ship the new key before signing with it.

## Release verification

Before treating a Linux release as stable, verify live composition on both X11 and Wayland: first load, Home/cancellation, resize/maximize/fullscreen, HiDPI, pointer and keyboard input, native dialog modality, minimize/restore, and translucent overlays during prolonged rendering. Container or offscreen checks do not establish physical-desktop or driver compatibility.
