// Runs cargo against the git-pinned core crate, or a checkout in LP_CORE_PATH.
// Use `pnpm run check:host`, `pnpm run test:host`, or `pnpm run resolve:host`.

import { spawnSync } from "node:child_process"
import fs from "node:fs"
import path from "node:path"

const CORE_GIT = "https://github.com/Arcadi4/litematica-preview-core"

const args = process.argv.slice(2)
const override = process.env.LP_CORE_PATH

if (!override) {
  const result = spawnSync("cargo", args, { stdio: "inherit" })
  process.exit(result.status ?? 1)
}

const checkout = path.resolve(override)
if (!fs.existsSync(path.join(checkout, "Cargo.toml"))) {
  console.error(`cargo: LP_CORE_PATH is not a core checkout: ${override}`)
  process.exit(1)
}

// `patch` takes the package directory, not its manifest. A Windows checkout
// arrives with backslashes that would otherwise escape the TOML string.
const escaped = checkout.replace(/\\/g, "\\\\").replace(/"/g, '\\"')
const patch = `patch."${CORE_GIT}".litematica-preview-core.path="${escaped}"`

// The patch replaces a git source, so the committed lock cannot describe it.
const cargoArgs = args.filter((arg) => arg !== "--locked")

const result = spawnSync("cargo", ["--config", patch, ...cargoArgs], { stdio: "inherit" })
process.exit(result.status ?? 1)
