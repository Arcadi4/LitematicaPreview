import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const target = process.env.CARGO_TARGET_DIR ?? path.join(root, "target")
const release = path.join(target, "release")
const portable = path.join(root, "artifacts", "linux-x64")
const binary = path.join(release, "LitematicaPreview")

if (process.platform !== "linux") {
  throw new Error(`Linux portable builds require Linux; current host is ${process.platform}.`)
}

const build = spawnSync(
  "cargo",
  ["build", "--release", "--locked", "--manifest-path", path.join(root, "src-tauri", "Cargo.toml")],
  { cwd: root, stdio: "inherit" },
)
if (build.status !== 0) {
  process.exit(build.status ?? 1)
}

if (!fs.existsSync(binary)) {
  throw new Error(`Release executable is missing: ${binary}`)
}

fs.rmSync(portable, { recursive: true, force: true })
fs.mkdirSync(path.join(portable, "Assets"), { recursive: true })
fs.mkdirSync(path.join(portable, "Demos"), { recursive: true })
fs.mkdirSync(path.join(portable, "Licenses", "ThirdParty"), { recursive: true })
fs.copyFileSync(binary, path.join(portable, "LitematicaPreview"))
fs.copyFileSync(path.join(root, "Assets", "pack.zip"), path.join(portable, "Assets", "pack.zip"))
fs.cpSync(path.join(root, "Fixtures", "Demos"), path.join(portable, "Demos"), { recursive: true })
fs.copyFileSync(path.join(root, "LICENSE"), path.join(portable, "Licenses", "LICENSE"))
fs.copyFileSync(path.join(root, "NOTICE"), path.join(portable, "Licenses", "NOTICE"))
fs.cpSync(path.join(root, "ThirdParty"), path.join(portable, "Licenses", "ThirdParty"), { recursive: true })

console.log(`Linux portable directory: ${portable}`)
