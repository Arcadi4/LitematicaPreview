// Builds the Linux x64 release artifacts: a Debian package and an AppImage.
// Run it through the package script: `pnpm run build:linux`.

import { spawn, spawnSync, type ChildProcess } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const TARGET = "x86_64-unknown-linux-gnu"
const TAURI_ROOT = path.join(ROOT, "src-tauri")
const ARTIFACTS_ROOT = path.join(ROOT, "artifacts")
const LOGS_ROOT = path.join(ARTIFACTS_ROOT, "logs")
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/
const MISSING_TOOL: Record<string, string> = {
  pnpm: "pnpm is not on PATH. Install Node.js 24 and the pnpm version declared in package.json, then open a new terminal.",
  cargo:
    "cargo is not on PATH. Install Rust from https://rustup.rs, reopen the terminal, then run: rustup target add x86_64-unknown-linux-gnu.",
}

interface TauriConfig {
  version: string
  frontendDist: string
  resources: Record<string, string>
}

let logPath = ""
let logStream: fs.WriteStream | undefined
let stage = "Preparing build log"
const startedAt = Date.now()

function log(message = ""): void {
  process.stdout.write(`${message}\n`)
  logStream?.write(`${message}\n`)
}

function logError(message: string): void {
  process.stderr.write(`${message}\n`)
  logStream?.write(`${message}\n`)
}

function elapsed(): string {
  const total = Math.round((Date.now() - startedAt) / 1000)
  const [hours, minutes, seconds] = [
    Math.floor(total / 3600),
    Math.floor((total % 3600) / 60),
    total % 60,
  ]
  return [hours, minutes, seconds].map((part) => String(part).padStart(2, "0")).join(":")
}

function resolveCommand(tool: string): { command: string; prefix: string[] } {
  if (tool === "pnpm") {
    const execPath = process.env.npm_execpath
    if (execPath && fs.existsSync(execPath)) {
      if (/\.[cm]?js$/i.test(execPath)) {
        return { command: process.execPath, prefix: [execPath] }
      }
      return { command: execPath, prefix: [] }
    }
  }
  return { command: tool, prefix: [] }
}

async function run(
  tool: string,
  args: string[],
  env: NodeJS.ProcessEnv = process.env,
): Promise<number> {
  const { command, prefix } = resolveCommand(tool)
  const child: ChildProcess = spawn(command, [...prefix, ...args], {
    cwd: ROOT,
    env,
    stdio: ["inherit", "pipe", "pipe"],
  })
  child.stdout?.on("data", (chunk: Buffer) => {
    process.stdout.write(chunk)
    logStream?.write(chunk)
  })
  child.stderr?.on("data", (chunk: Buffer) => {
    process.stderr.write(chunk)
    logStream?.write(chunk)
  })
  const { promise, resolve, reject } = Promise.withResolvers<number>()
  child.on("error", (cause: Error) => {
    reject(new Error(MISSING_TOOL[tool] ?? cause.message))
  })
  child.on("close", (code, signal) => {
    if (code === null) {
      reject(new Error(`${tool} was terminated by signal ${signal ?? "unknown"}`))
      return
    }
    resolve(code)
  })
  return await promise
}

function capture(tool: string, args: string[]): string {
  const { command, prefix } = resolveCommand(tool)
  const result = spawnSync(command, [...prefix, ...args], {
    cwd: ROOT,
    encoding: "utf8",
  })
  if (result.error) {
    throw new Error(MISSING_TOOL[tool] ?? result.error.message)
  }
  if (result.status !== 0) {
    throw new Error(
      `${tool} ${args.join(" ")} failed: ${result.stderr.trim() || `exit code ${result.status}`}`,
    )
  }
  return result.stdout.trim()
}

function readConfig(): TauriConfig {
  const parsed: unknown = JSON.parse(
    fs.readFileSync(path.join(TAURI_ROOT, "tauri.conf.json"), "utf8"),
  )
  if (typeof parsed !== "object" || parsed === null) {
    throw new Error("src-tauri/tauri.conf.json must contain a configuration object.")
  }
  const record = parsed as Record<string, unknown>
  if (typeof record.version !== "string" || !SEMVER.test(record.version)) {
    throw new Error(
      "src-tauri/tauri.conf.json must contain an explicit semantic version for package filenames.",
    )
  }
  const build = record.build
  const frontendDist =
    typeof build === "object" && build !== null && "frontendDist" in build
      ? build.frontendDist
      : undefined
  if (typeof frontendDist !== "string") {
    throw new Error("src-tauri/tauri.conf.json must declare build.frontendDist.")
  }
  const resources: Record<string, string> = {}
  const bundle = record.bundle
  const declared =
    typeof bundle === "object" && bundle !== null && "resources" in bundle
      ? bundle.resources
      : undefined
  if (typeof declared === "object" && declared !== null && !Array.isArray(declared)) {
    for (const [resource, destination] of Object.entries(declared)) {
      if (typeof destination !== "string" || destination.trim() === "") {
        throw new Error(
          `src-tauri/tauri.conf.json maps ${resource} to an invalid bundle destination.`,
        )
      }
      resources[resource] = destination
    }
  }
  if (Object.keys(resources).length === 0) {
    throw new Error("src-tauri/tauri.conf.json must declare bundle resources.")
  }
  return { version: record.version, frontendDist, resources }
}

function requireFile(target: string, describe: string): void {
  let stats: fs.Stats
  try {
    stats = fs.statSync(target)
  } catch {
    throw new Error(`${describe} is missing: ${target}`)
  }
  if (!stats.isFile()) {
    throw new Error(`${describe} is not a file: ${target}`)
  }
  if (stats.size === 0) {
    throw new Error(`${describe} is empty: ${target}`)
  }
}

function verifyResourceInputs(resources: Record<string, string>): void {
  for (const source of Object.keys(resources)) {
    const absolute = path.join(TAURI_ROOT, source)
    let stats: fs.Stats
    try {
      stats = fs.statSync(absolute)
    } catch {
      throw new Error(
        `Required bundle resource is missing: ${absolute}. Restore the tracked input before building.`,
      )
    }
    if (stats.isDirectory()) {
      if (!fs.readdirSync(absolute).length) {
        throw new Error(`Required bundle resource directory is empty: ${absolute}`)
      }
    } else if (!stats.isFile()) {
      throw new Error(`Required bundle resource is not a file or directory: ${absolute}`)
    } else if (stats.size === 0) {
      throw new Error(`Required bundle resource is empty: ${absolute}`)
    }
  }
}

function findBundle(root: string, extension: string, describe: string): string {
  if (!fs.existsSync(root)) {
    throw new Error(`The ${describe} output directory is missing: ${root}`)
  }
  const matches = fs
    .readdirSync(root)
    .filter((name) => name.endsWith(extension))
    .map((name) => path.join(root, name))
  if (matches.length !== 1) {
    throw new Error(`Expected one fresh ${describe} in ${root}; found ${matches.length}.`)
  }
  return matches[0]
}

function describePackage(target: string): string {
  const { size } = fs.statSync(target)
  return `${target}\n  ${(size / 1024 / 1024).toFixed(2)} MiB (${size.toLocaleString("en-US")} bytes)`
}

async function main(): Promise<void> {
  if (process.platform !== "linux" || process.arch !== "x64") {
    throw new Error(
      `Linux x64 packages can only be produced on Linux x64, but this host is ${process.platform} ${process.arch}. ` +
        "Use Ubuntu 24.04 or another Debian-based distribution with GTK 3, WebKitGTK 4.1, and the Rust target.",
    )
  }

  fs.mkdirSync(LOGS_ROOT, { recursive: true })
  const stamp = new Date().toISOString().replace(/[:.]/g, "-")
  logPath = path.join(LOGS_ROOT, `build-linux-${stamp}.log`)
  logStream = fs.createWriteStream(logPath, { flags: "a" })
  log("Litematica Preview - Linux x64 release build")
  log(`Build log: ${logPath}`)

  stage = "Checking prerequisites"
  log(`\n[1/4] ${stage}`)
  log(`Node.js ${process.version}`)
  log(`pnpm ${capture("pnpm", ["--version"])}`)
  log(capture("cargo", ["--version"]))
  log(
    `Requires libwebkit2gtk-4.1-dev, libgtk-3-dev, libxdo-dev, librsvg2-dev, patchelf, and: rustup target add ${TARGET}`,
  )

  const config = readConfig()
  verifyResourceInputs(config.resources)
  const packageName = `LitematicaPreview-${config.version}-linux-x64`
  const targetDir = path.resolve(ROOT, process.env.CARGO_TARGET_DIR ?? "target")
  const releaseRoot = path.join(targetDir, TARGET, "release")
  const debRoot = path.join(releaseRoot, "bundle", "deb")
  const appImageRoot = path.join(releaseRoot, "bundle", "appimage")
  const debPath = path.join(ARTIFACTS_ROOT, `${packageName}.deb`)
  const appImagePath = path.join(ARTIFACTS_ROOT, `${packageName}.AppImage`)

  stage = "Building the application, Debian package, and AppImage"
  log(`\n[2/4] ${stage}`)
  for (const stale of [debRoot, appImageRoot]) {
    fs.rmSync(stale, { force: true, recursive: true })
  }
  for (const stale of [debPath, appImagePath]) {
    fs.rmSync(stale, { force: true })
  }
  const tauri = await run(
    "pnpm",
    [
      "exec",
      "tauri",
      "build",
      "--target",
      TARGET,
      "--ci",
      "--bundles",
      "deb,appimage",
      "--",
      "--locked",
    ],
    { ...process.env, CARGO_TARGET_DIR: targetDir },
  )
  if (tauri !== 0) {
    throw new Error(
      "Tauri release build failed. Check the compiler and bundler output in the log; confirm libwebkit2gtk-4.1-dev, libgtk-3-dev, libxdo-dev, librsvg2-dev, patchelf, and the Rust target are installed.",
    )
  }
  requireFile(
    path.resolve(TAURI_ROOT, config.frontendDist, "index.html"),
    "Built frontend entry point",
  )

  stage = "Staging the Debian package and AppImage"
  log(`\n[3/4] ${stage}`)
  requireFile(path.join(releaseRoot, "LitematicaPreview"), "Release executable")
  const deb = findBundle(debRoot, ".deb", "Debian package")
  const appImage = findBundle(appImageRoot, ".AppImage", "AppImage")
  fs.copyFileSync(deb, debPath)
  fs.copyFileSync(appImage, appImagePath)
  fs.chmodSync(appImagePath, 0o755)

  stage = "Verifying the generated packages"
  log(`\n[4/4] ${stage}`)
  for (const generated of [debPath, appImagePath]) {
    requireFile(generated, "Generated package")
  }
  log(`\nBuild completed in ${elapsed()}.`)
  for (const generated of [debPath, appImagePath]) {
    log(describePackage(generated))
  }
  log(`Build log: ${logPath}`)
  log(`Install the Debian package: sudo apt install ./${path.basename(debPath)}`)
  log(
    `Run the AppImage: chmod +x ${path.basename(appImagePath)} && ./${path.basename(appImagePath)}`,
  )
}

try {
  await main()
} catch (cause) {
  logError(`\nBuild failed during: ${stage}`)
  logError(`Elapsed: ${elapsed()}`)
  logError(`Reason: ${(cause as Error).message}`)
  if (logPath) {
    logError(`Build log: ${logPath}`)
  }
  process.exitCode = 1
}

if (logStream) {
  const stream = logStream
  logStream = undefined
  const { promise, resolve } = Promise.withResolvers<void>()
  stream.end(() => resolve())
  await promise
}
