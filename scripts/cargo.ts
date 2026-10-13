// Runs cargo against the published core crate. Set LP_CORE_PATH to a checkout of
// https://github.com/Arcadi4/litematica-preview-core to build that one instead,
// which is how core changes are exercised in the host before a release.
//
// This mirrors litematicaql's Core/cargo.sh, so both consumers opt into local
// core development the same way.

import { spawnSync } from "node:child_process"
import fs from "node:fs"
import path from "node:path"

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

// `patch.crates-io` takes the package directory, not its manifest. A Windows
// checkout arrives with backslashes that would otherwise escape the TOML string.
const escaped = checkout.replace(/\\/g, "\\\\").replace(/"/g, '\\"')
const patch = `patch.crates-io.litematica-preview-core.path="${escaped}"`

const result = spawnSync("cargo", ["--config", patch, ...args], { stdio: "inherit" })
process.exit(result.status ?? 1)
