import { execFileSync } from "node:child_process"
import path from "node:path"

// Exercise exactly the source archive a registry consumer will receive.
const manifest = path.resolve("crates/core/Cargo.toml")
const metadata = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--manifest-path", manifest, "--no-deps", "--format-version", "1"],
    { encoding: "utf8" },
  ),
) as {
  target_directory: string
  packages: { name: string; version: string; manifest_path: string }[]
}
const core = metadata.packages.find((pkg) => pkg.manifest_path === manifest)
if (!core) throw new Error("The core package is missing from workspace metadata.")

execFileSync("cargo", ["package", "--manifest-path", manifest, "--locked", "--allow-dirty"], {
  stdio: "inherit",
})
const packagedManifest = path.join(
  metadata.target_directory,
  "package",
  `${core.name}-${core.version}`,
  "Cargo.toml",
)
execFileSync("cargo", ["test", "--manifest-path", packagedManifest, "--lib", "--locked"], {
  stdio: "inherit",
})
