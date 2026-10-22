// Writes `SHA256SUMS.txt` for the release packages; run `pnpm run checksums`.
// Pass `--win`, `--linux`, or both (default: `--win`).

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

const PLATFORM_FLAGS: Record<string, true> = {
  "--win": true,
  "--linux": true,
}

function platforms(): string[] {
  const requested = process.argv.slice(2)
  const selected = requested.length === 0 ? ["--win"] : requested
  for (const platform of selected) {
    if (PLATFORM_FLAGS[platform] !== true) {
      throw new Error(`Unsupported checksum platform: ${platform}. Use --win or --linux.`)
    }
  }
  return selected
}

function artifactNames(version: string, selected: readonly string[]): string[] {
  const names: string[] = []
  if (selected.includes("--win")) {
    const packageName = `LitematicaPreview-${version}-win-x64`
    names.push(`${packageName}-setup.exe`, `${packageName}-portable.zip`)
  }
  if (selected.includes("--linux")) {
    const packageName = `LitematicaPreview-${version}-linux-x64`
    names.push(`${packageName}.deb`, `${packageName}.AppImage`)
  }
  return names
}

function main(): void {
  const lines = artifactNames(packageVersion(), platforms()).map((name) => {
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
