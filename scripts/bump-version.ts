#!/usr/bin/env node

// Tags a release: bump the version, commit, and tag.
//
// Takes the component to raise, so the current version never has to be looked
// up: a bare `pnpm run version:bump` and `pnpm run version:bump patch` both give
// 0.3.1 from 0.3.0, `pnpm run version:bump minor` gives 0.4.0, and
// `pnpm run version:bump major` gives 1.0.0. Runs only on main from a clean
// workspace, so the tag always names exactly what was reviewed. Push the
// commit and the tag yourself; the release workflow fires on the tag.

import { execSync } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const ROOT_DIR = path.resolve(__dirname, "..")

interface VersionLocation {
  id: string
  file: string
  read: (filePath: string) => string | null
  write: (filePath: string, newVersion: string) => void
}

interface LocationResult extends VersionLocation {
  version: string | null
  error: string | null
}

interface BumpOptions {
  dryRun?: boolean
}

interface ParsedSemver {
  raw: string
  major: number
  minor: number
  patch: number
  prerelease: string | null
  build: string | null
  normalized: string
}

interface CheckOptions {
  quiet?: boolean
}

const LOCATIONS: VersionLocation[] = [
  {
    id: "package.json",
    file: path.join(ROOT_DIR, "package.json"),
    read(file: string): string | null {
      const json = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: string }
      return json.version ?? null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(/(^  "version"\s*:\s*)"[^"]+"/m, `$1"${newVersion}"`)
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
  {
    id: "src-tauri/tauri.conf.json",
    file: path.join(ROOT_DIR, "src-tauri/tauri.conf.json"),
    read(file: string): string | null {
      const json = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: string }
      return json.version ?? null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(/(^  "version"\s*:\s*)"[^"]+"/m, `$1"${newVersion}"`)
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
  {
    id: "src-tauri/Cargo.toml",
    file: path.join(ROOT_DIR, "src-tauri/Cargo.toml"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8")
      const match = content.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)
      return match ? match[1] : null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(
        /(^\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
        `$1"${newVersion}"`,
      )
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
  {
    id: "crates/core/Cargo.toml",
    file: path.join(ROOT_DIR, "crates/core/Cargo.toml"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8")
      const match = content.match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)
      return match ? match[1] : null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(
        /(^\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
        `$1"${newVersion}"`,
      )
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
  {
    id: "Cargo.lock (litematica-preview)",
    file: path.join(ROOT_DIR, "Cargo.lock"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8")
      const match = content.match(
        /\[\[package\]\]\s+name\s*=\s*"litematica-preview"\s+version\s*=\s*"([^"]+)"/m,
      )
      return match ? match[1] : null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(
        /(\[\[package\]\]\s+name\s*=\s*"litematica-preview"\s+version\s*=\s*)"[^"]+"/g,
        `$1"${newVersion}"`,
      )
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update litematica-preview version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
  {
    id: "Cargo.lock (litematica-preview-native)",
    file: path.join(ROOT_DIR, "Cargo.lock"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8")
      const match = content.match(
        /\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*"([^"]+)"/m,
      )
      return match ? match[1] : null
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8")
      const updated = raw.replace(
        /(\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*)"[^"]+"/g,
        `$1"${newVersion}"`,
      )
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update litematica-preview-native version in ${file}`)
      }
      fs.writeFileSync(file, updated, "utf8")
    },
  },
]

const SEMVER_REGEX = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/

function parseSemver(str: string): ParsedSemver | null {
  const match = String(str).trim().match(SEMVER_REGEX)
  if (!match) return null
  return {
    raw: str,
    major: parseInt(match[1], 10),
    minor: parseInt(match[2], 10),
    patch: parseInt(match[3], 10),
    prerelease: match[4] || null,
    build: match[5] || null,
    normalized: `${match[1]}.${match[2]}.${match[3]}${
      match[4] ? `-${match[4]}` : ""
    }${match[5] ? `+${match[5]}` : ""}`,
  }
}

function calculateBump(currentVersion: string, bumpType: string): string | null {
  const parsed = parseSemver(currentVersion)
  if (!parsed) {
    throw new Error(`Cannot bump invalid semver version: "${currentVersion}"`)
  }
  const { major, minor, patch } = parsed
  switch (bumpType.toLowerCase()) {
    case "major":
      return `${major + 1}.0.0`
    case "minor":
      return `${major}.${minor + 1}.0`
    case "patch":
      return `${major}.${minor}.${patch + 1}`
    default:
      return null
  }
}

function readAllVersions(): LocationResult[] {
  const results: LocationResult[] = []
  for (const loc of LOCATIONS) {
    if (!fs.existsSync(loc.file)) {
      results.push({ ...loc, version: null, error: "File not found" })
      continue
    }
    try {
      const ver = loc.read(loc.file)
      results.push({ ...loc, version: ver, error: ver ? null : "Could not extract version" })
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err)
      results.push({ ...loc, version: null, error: msg })
    }
  }
  return results
}

function reportError(msg: string): void {
  const prefix = process.env.GITHUB_ACTIONS ? "::error::" : "Error: "
  console.error(`${prefix}${msg}`)
}

function bumpError(msg: string): void {
  console.error(`bump: ${msg}`)
}

function checkVersions(expectedTagOrVersion?: string | null, options: CheckOptions = {}): boolean {
  const versions = readAllVersions()
  const errors: string[] = []

  for (const v of versions) {
    if (v.error) {
      errors.push(`${v.id}: ${v.error}`)
    }
  }

  const distinctVersions = Object.keys(
    Object.fromEntries(versions.filter((v) => v.version).map((v) => [v.version as string, true])),
  )

  if (distinctVersions.length === 0) {
    reportError("No version numbers found across project files.")
    return false
  }

  if (distinctVersions.length > 1) {
    const lines = [
      "Version mismatch across project files:",
      ...versions.map((v) => `  - ${v.id}: ${v.version || `(${v.error})`}`),
    ]
    reportError(lines.join("\n"))
    return false
  }

  const currentVersion = distinctVersions[0]

  if (expectedTagOrVersion) {
    const rawExpected = String(expectedTagOrVersion).trim()
    const expected = rawExpected.startsWith("v") ? rawExpected.slice(1) : rawExpected
    const expectedBase = expected.split("-")[0]

    if (currentVersion !== expected && currentVersion !== expectedBase) {
      reportError(
        `Version mismatch: target is '${rawExpected}' (version '${expected}'), but project files define '${currentVersion}'`,
      )
      return false
    }

    if (!options.quiet) {
      console.log(
        `✓ All ${LOCATIONS.length} project locations match version ${currentVersion} (tag '${rawExpected}').`,
      )
    }
  } else if (!options.quiet) {
    console.log(`✓ All ${LOCATIONS.length} project locations match version ${currentVersion}.`)
  }

  return true
}

function bumpVersions(level: string = "patch", options: BumpOptions = {}): boolean {
  const { dryRun = false } = options

  let branch = ""
  try {
    branch = execSync("git branch --show-current", { cwd: ROOT_DIR, encoding: "utf8" }).trim()
  } catch {
    // ignore if not a git repository
  }

  const allowedBranch = process.env.BUMP_ALLOW_BRANCH || "main"
  if (branch !== allowedBranch) {
    if (dryRun) {
      console.warn(`[dry-run] Warning: versions are cut from main, not ${branch || "(unknown)"}`)
    } else {
      bumpError(`versions are cut from main, not ${branch || "(unknown)"}`)
      return false
    }
  }

  let porcelain = ""
  try {
    porcelain = execSync("git status --porcelain", { cwd: ROOT_DIR, encoding: "utf8" }).trim()
  } catch {
    // ignore if not a git repository
  }

  if (porcelain.length > 0) {
    if (dryRun) {
      console.warn("[dry-run] Warning: workspace has uncommitted changes:")
      const shortStatus = execSync("git status --short", { cwd: ROOT_DIR, encoding: "utf8" })
      process.stderr.write(shortStatus)
    } else {
      bumpError("workspace has uncommitted changes:")
      const shortStatus = execSync("git status --short", { cwd: ROOT_DIR, encoding: "utf8" })
      process.stderr.write(shortStatus)
      return false
    }
  }

  const versions = readAllVersions()
  const distinctVersions = Object.keys(
    Object.fromEntries(versions.filter((v) => v.version).map((v) => [v.version as string, true])),
  )

  if (distinctVersions.length === 0) {
    bumpError("no version numbers found across project files")
    return false
  }

  if (distinctVersions.length > 1) {
    const lines = [
      "version mismatch across project files:",
      ...versions.map((v) => `  - ${v.id}: ${v.version || `(${v.error})`}`),
    ]
    bumpError(lines.join("\n"))
    return false
  }

  const current = distinctVersions[0]

  const normalizedLevel = level.trim().toLowerCase()
  let next: string | null = null

  if (normalizedLevel === "major" || normalizedLevel === "minor" || normalizedLevel === "patch") {
    next = calculateBump(current, normalizedLevel)
  } else {
    const parsed = parseSemver(level)
    if (parsed) {
      next = parsed.normalized
    }
  }

  if (!next) {
    console.error("usage: pnpm run version:bump major|minor|patch")
    return false
  }

  try {
    execSync(`git rev-parse -q --verify "refs/tags/v${next}"`, {
      cwd: ROOT_DIR,
      stdio: "ignore",
    })
    bumpError(`tag v${next} already exists`)
    return false
  } catch {
    // Tag does not exist, which is expected
  }

  const expectedFiles: string[] = []
  for (const loc of LOCATIONS) {
    const rel = path.relative(ROOT_DIR, loc.file).split(path.sep).join("/")
    if (!expectedFiles.includes(rel)) {
      expectedFiles.push(rel)
    }
  }
  expectedFiles.sort()

  if (dryRun) {
    console.log(`[dry-run] Would bump version: ${current} -> ${next}`)
    for (const v of versions) {
      console.log(`[dry-run]   ${v.id}: ${v.version || current} -> ${next}`)
    }
    console.log(`[dry-run] Would stage: ${expectedFiles.join(" ")}`)
    console.log(`[dry-run] Would commit: chore: bump to v${next}`)
    console.log(`[dry-run] Would tag: v${next}`)
    console.log(`bumped ${current} to ${next}, committed and tagged v${next}`)
    console.log(`publish with: git push origin main v${next}`)
    return true
  }

  for (const loc of LOCATIONS) {
    loc.write(loc.file, next)
  }

  const check = checkVersions(next, { quiet: true })
  if (!check) {
    bumpError("refusing to commit changes: post-bump verification failed")
    return false
  }

  const changedOutput = execSync("git diff --name-only", { cwd: ROOT_DIR, encoding: "utf8" })
  const changed = changedOutput
    .split(/\r?\n/)
    .map((s) => s.trim().replace(/\\/g, "/"))
    .filter(Boolean)
    .sort()

  const matches =
    changed.length === expectedFiles.length &&
    changed.every((file, index) => file === expectedFiles[index])

  if (!matches) {
    bumpError("refusing to commit changes beyond the version bump:")
    const shortStatus = execSync("git status --short", { cwd: ROOT_DIR, encoding: "utf8" })
    process.stderr.write(shortStatus)
    return false
  }

  try {
    execSync(`git add ${expectedFiles.map((f) => `"${f}"`).join(" ")}`, {
      cwd: ROOT_DIR,
      stdio: "inherit",
    })
    execSync(`git commit -m "chore: bump to v${next}"`, {
      cwd: ROOT_DIR,
      stdio: "inherit",
    })
    execSync(`git tag -a "v${next}" -m "v${next}"`, {
      cwd: ROOT_DIR,
      stdio: "inherit",
    })
  } catch {
    bumpError("git commit or tag failed")
    return false
  }

  console.log(`bumped ${current} to ${next}, committed and tagged v${next}`)
  console.log(`publish with: git push origin main v${next}`)

  return true
}

function showHelp(): void {
  console.log(`
Usage:
  pnpm run version:bump [major | minor | patch] [options]
  pnpm run version:check [tag_or_version]

Arguments:
  major | minor | patch   Bump the corresponding semver component (default: patch)

Options:
  --check, -c             Verify that all locations match each other and optional expected tag/version
  --dry-run, -n           Simulate the bump without modifying files or committing
  --help, -h              Show this help message

Locations updated:
  - package.json
  - src-tauri/tauri.conf.json
  - src-tauri/Cargo.toml
  - crates/core/Cargo.toml
  - Cargo.lock (litematica-preview)
  - Cargo.lock (litematica-preview-native)
`)
}

function main(): void {
  const args = process.argv.slice(2)
  if (args.includes("--help") || args.includes("-h")) {
    showHelp()
    const versions = readAllVersions()
    console.log("Current versions:")
    for (const v of versions) {
      console.log(`  ${v.id}: ${v.version || `(${v.error})`}`)
    }
    process.exit(0)
  }

  const checkIndex = args.findIndex((a) => a === "--check" || a === "-c")
  if (checkIndex !== -1) {
    const remaining = args.filter((_, i) => i !== checkIndex)
    const expected = remaining[0] || null
    const ok = checkVersions(expected)
    process.exit(ok ? 0 : 1)
  }

  const dryRun = args.includes("--dry-run") || args.includes("-n")
  const targetArg = args.find((a) => !a.startsWith("-")) || "patch"

  const ok = bumpVersions(targetArg, { dryRun })
  process.exit(ok ? 0 : 1)
}

main()
