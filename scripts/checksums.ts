// Writes SHA256SUMS.txt for the generated release packages. Run it through the
// package script: `pnpm run checksums`.

import { createHash } from "node:crypto"
import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const ARTIFACTS_ROOT = path.join(ROOT, "artifacts")

function packageVersion(): string {
  const parsed: unknown = JSON.parse(
    fs.readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"),
  )
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    !("version" in parsed) ||
    typeof parsed.version !== "string"
  ) {
    throw new Error("src-tauri/tauri.conf.json must contain a string version.")
  }
  return parsed.version
}

function main(): void {
  const packageName = `LitematicaPreview-${packageVersion()}-win-x64`
  const lines = [`${packageName}-setup.exe`, `${packageName}-portable.zip`].map((name) => {
    const target = path.join(ARTIFACTS_ROOT, name)
    if (!fs.existsSync(target)) {
      throw new Error(`Release package not found: ${target}`)
    }
    return `${createHash("sha256").update(fs.readFileSync(target)).digest("hex")}  ${name}`
  })

  const checksumPath = path.join(ARTIFACTS_ROOT, "SHA256SUMS.txt")
  fs.writeFileSync(checksumPath, `${lines.join("\n")}\n`)
  console.log(`Generating SHA256SUMS.txt for release assets in ${checksumPath}:`)
  console.log(lines.join("\n"))
}

try {
  main()
} catch (cause) {
  console.error(`Error: ${(cause as Error).message}`)
  process.exitCode = 1
}
