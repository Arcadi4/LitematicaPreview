# Development and release tasks. Run `just --list`.
#
# Recipes run in the justfile's directory, which is the repository root.

set positional-arguments

app := "litematica-preview"

# A local core checkout overrides the crates.io dependency for development.
# The path is TOML-escaped, then shell-quoted as one argument. `--locked` is
# dropped while the override is active because the lock cannot describe it.
core_path := env("LP_CORE_PATH", "")
core_patch := if core_path == "" {
    ""
} else {
    "--config=patch.crates-io.litematica-preview-core.path=" + quote("\"" + replace(replace(core_path, "\\", "\\\\"), "\"", "\\\"") + "\"")
}
locked := if core_path == "" { "--locked" } else { "" }

default:
    @just --list

dev *args:
    cargo {{core_patch}} run -p {{app}} {{locked}} -- "$@"

open *paths:
    cargo {{core_patch}} run -p {{app}} {{locked}} -- --open "$@"

build:
    cargo {{core_patch}} build -p {{app}} {{locked}}

fmt:
    cargo fmt --all

lint:
    cargo {{core_patch}} clippy --workspace --all-targets {{locked}} -- -D warnings

test:
    cargo {{core_patch}} test --workspace {{locked}}

check:
    cargo fmt --all -- --check
    cargo {{core_patch}} clippy --workspace --all-targets {{locked}} -- -D warnings
    cargo {{core_patch}} test --workspace {{locked}}

package-windows *args:
    cargo xtask package-windows "$@"

package-linux *args:
    cargo xtask package-linux "$@"

package-macos *args:
    cargo xtask package-macos "$@"

publish-homebrew *args:
    cargo xtask publish-homebrew "$@"

apt-publish *args:
    @case "${1:-}" in --deb|--help|-h|"") cargo xtask apt-publish "$@" ;; *) cargo xtask apt-publish --deb "$@" ;; esac

release-check *args:
    cargo xtask release-check "$@"

release-publish *args:
    cargo xtask release-publish "$@"

release-apt *args:
    cargo xtask release-apt "$@"

checksums *args:
    cargo xtask checksums "$@"

version *args:
    cargo xtask version "$@"
