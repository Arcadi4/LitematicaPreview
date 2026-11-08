// Builds the Windows x64 release artifacts with Velopack: a one-click setup
// executable, a portable ZIP, and the update feed uploaded to R2. Run it
// through the package script: `pnpm run build:win`.

import { spawn, spawnSync, type ChildProcess } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const TARGET = "x86_64-pc-windows-msvc"
const TAURI_ROOT = path.join(ROOT, "src-tauri")
const ARTIFACTS_ROOT = path.join(ROOT, "artifacts")
const LOGS_ROOT = path.join(ARTIFACTS_ROOT, "logs")
const VELOPACK_ROOT = path.join(ARTIFACTS_ROOT, "velopack")
const PACK_ID = "LitematicaPreview"
const CHANNEL = "win"
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/
const REQUIRED_FILES = [
  "LitematicaPreview.exe",
  "Assets/pack.zip",
  "Licenses/LICENSE",
  "Licenses/NOTICE",
]
const REQUIRED_DIRECTORIES = ["Assets", "Demos", "Licenses", "Licenses/ThirdParty"]
const MISSING_TOOL: Record<string, string> = {
  pnpm: "pnpm is not on PATH. Install Node.js 24, run: npm install --global pnpm@12.5.1, then open a new terminal.",
  cargo:
    "cargo is not on PATH. Install Rust from https://rustup.rs, reopen the terminal, then run: rustup target add x86_64-pc-windows-msvc.",
  vpk: "vpk is not on PATH. Install the .NET 10 SDK, then run: dotnet tool install --global vpk --version 1.2.161, and open a new terminal.",
}

interface TauriConfig {
  version: string
  bundle: { resources: Record<string, string> }
}

interface Signing {
  args: string[]
  describe: string
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
    // Running through a package script keeps pnpm resolution package-manager agnostic.
    const execPath = process.env.npm_execpath
    if (execPath && fs.existsSync(execPath)) {
      if (path.extname(execPath).toLowerCase() === ".exe") {
        return { command: execPath, prefix: [] }
      }
      return { command: process.execPath, prefix: [execPath] }
    }
  }
  return {
    command: process.platform === "win32" && tool === "pnpm" ? "pnpm.cmd" : tool,
    prefix: [],
  }
}

async function run(
  tool: string,
  args: string[],
  env: NodeJS.ProcessEnv = process.env,
): Promise<number> {
  const { command, prefix } = resolveCommand(tool)
  const child: ChildProcess = spawn(command, [...prefix, ...args], {
    env,
    stdio: ["inherit", "pipe", "pipe"],
    windowsHide: true,
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
  const result = spawnSync(command, [...prefix, ...args], { encoding: "utf8", windowsHide: true })
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

function isInteractive(): boolean {
  return (
    process.stdin.isTTY === true &&
    process.stdout.isTTY === true &&
    !process.env.CI &&
    !process.env.GITHUB_ACTIONS &&
    !process.env.TF_BUILD &&
    !process.argv.includes("--non-interactive")
  )
}

function prompt(question: string): Promise<string> {
  const { promise, resolve } = Promise.withResolvers<string>()
  process.stdout.write(question)
  process.stdin.resume()
  process.stdin.setEncoding("utf8")
  process.stdin.once("data", (answer: string) => {
    process.stdin.pause()
    resolve(answer)
  })
  return promise
}

function readConfig(): TauriConfig {
  const parsed: unknown = JSON.parse(
    fs.readFileSync(path.join(TAURI_ROOT, "tauri.conf.json"), "utf8"),
  )
  if (
    typeof parsed !== "object" ||
    parsed === null ||
    !("version" in parsed) ||
    typeof parsed.version !== "string" ||
    !SEMVER.test(parsed.version)
  ) {
    throw new Error(
      "src-tauri/tauri.conf.json must contain an explicit semantic version for package filenames.",
    )
  }
  const resources: Record<string, string> = {}
  const bundle = "bundle" in parsed ? parsed.bundle : undefined
  const declared =
    typeof bundle === "object" && bundle !== null && "resources" in bundle
      ? bundle.resources
      : undefined
  if (typeof declared === "object" && declared !== null) {
    for (const [resource, destination] of Object.entries(declared)) {
      if (typeof destination !== "string") {
        throw new Error(
          `src-tauri/tauri.conf.json maps ${resource} to a non-string bundle destination.`,
        )
      }
      resources[resource] = destination
    }
  }
  return { version: parsed.version, bundle: { resources } }
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

function assertPortableIsUnlocked(portableRoot: string): void {
  const executable = path.join(portableRoot, "LitematicaPreview.exe")
  if (!fs.existsSync(executable)) {
    return
  }
  try {
    // A running application maps its own image, so the write probe fails while it holds the file.
    fs.closeSync(fs.openSync(executable, "r+"))
  } catch {
    throw new Error(`Close the portable Litematica Preview app before building: ${executable}`)
  }
}

function stageResources(resources: Record<string, string>, portableRoot: string): void {
  for (const [resource, destination] of Object.entries(resources)) {
    const source = path.join(TAURI_ROOT, resource)
    if (!fs.existsSync(source)) {
      throw new Error(
        `Required bundle resource is missing: ${source}. Restore the tracked input before building.`,
      )
    }
    const target = path.join(portableRoot, destination)
    fs.rmSync(target, { force: true, recursive: true })
    if (fs.statSync(source).isDirectory()) {
      fs.cpSync(source, target, { force: true, recursive: true })
    } else {
      fs.mkdirSync(path.dirname(target), { recursive: true })
      fs.copyFileSync(source, target)
    }
  }
}

function seedPreviousReleases(): void {
  const previous = process.env.VELOPACK_PREVIOUS_RELEASES?.trim()
  if (!previous) {
    return
  }
  if (!fs.existsSync(previous) || !fs.statSync(previous).isDirectory()) {
    throw new Error(`VELOPACK_PREVIOUS_RELEASES is not a directory: ${previous}`)
  }
  for (const entry of fs.readdirSync(previous, { withFileTypes: true })) {
    if (entry.isFile()) {
      fs.copyFileSync(path.join(previous, entry.name), path.join(VELOPACK_ROOT, entry.name))
    }
  }
  log(`Copied previous release assets from ${previous} for delta generation.`)
}

function requiredSigning(): Signing | null {
  const params = process.env.LITEMATICA_WINDOWS_SIGN_PARAMS?.trim()
  const azure = process.env.LITEMATICA_WINDOWS_AZURE_SIGN_FILE?.trim()
  if (params && azure) {
    throw new Error(
      "Set only one of LITEMATICA_WINDOWS_SIGN_PARAMS and LITEMATICA_WINDOWS_AZURE_SIGN_FILE.",
    )
  }
  if (params) {
    return { args: ["--signParams", params], describe: "signtool parameters" }
  }
  if (azure) {
    requireFile(azure, "Azure Artifact Signing metadata")
    return {
      args: ["--azureTrustedSignFile", azure],
      describe: "Azure Artifact Signing",
    }
  }
  const allowed =
    process.env.LITEMATICA_ALLOW_UNSIGNED === "1" || process.argv.includes("--allow-unsigned")
  if (allowed) {
    return null
  }
  throw new Error(
    "Windows packages must be code-signed before Velopack distributes Setup.exe and Update.exe. " +
      "Set LITEMATICA_WINDOWS_SIGN_PARAMS or LITEMATICA_WINDOWS_AZURE_SIGN_FILE. " +
      "For a local unsigned test build only, pass --allow-unsigned or set LITEMATICA_ALLOW_UNSIGNED=1.",
  )
}

function verifySignature(target: string): void {
  const script = [
    `$signature = Get-AuthenticodeSignature -LiteralPath '${target.replaceAll("'", "''")}'`,
    "if ($signature.Status -ne 'Valid') { Write-Error \"Unsigned package: $($signature.Status)\"; exit 1 }",
  ].join("; ")
  const result = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
    encoding: "utf8",
    windowsHide: true,
  })
  if (result.error || result.status !== 0) {
    throw new Error(
      `The package is not validly signed: ${target}\n${result.stderr.trim() || result.error?.message || ""}`,
    )
  }
}

function findPackaged(directory: string, suffix: string, describe: string): string {
  const matches = fs
    .readdirSync(directory)
    .filter((name) => name.endsWith(suffix))
    .map((name) => path.join(directory, name))
  if (matches.length !== 1) {
    throw new Error(`Expected one ${describe} in ${directory}; found ${matches.length}.`)
  }
  return matches[0]
}

function describePackage(target: string): string {
  const { size } = fs.statSync(target)
  return `${target}\n  ${(size / 1024 / 1024).toFixed(2)} MiB (${size.toLocaleString("en-US")} bytes)`
}

function openArtifactsFolder(): void {
  const opener =
    process.platform === "win32" ? "explorer" : process.platform === "darwin" ? "open" : "xdg-open"
  const child = spawn(opener, [ARTIFACTS_ROOT], {
    detached: true,
    stdio: "ignore",
    windowsHide: true,
  })
  child.on("error", (cause: Error) => {
    log(`The build succeeded, but the artifacts folder could not be opened: ${cause.message}`)
  })
  child.unref()
}

async function main(): Promise<void> {
  fs.mkdirSync(LOGS_ROOT, { recursive: true })
  const stamp = new Date().toISOString().replace(/[:.]/g, "-")
  logPath = path.join(LOGS_ROOT, `build-${stamp}.log`)
  logStream = fs.createWriteStream(logPath, { flags: "a" })
  log("Litematica Preview - Windows x64 Velopack release build")
  log(`Build log: ${logPath}`)

  stage = "Checking prerequisites"
  log(`\n[1/6] ${stage}`)
  if (process.platform !== "win32") {
    throw new Error(
      `Windows x64 packages can only be produced on Windows, but this host is ${process.platform}. ` +
        "Use Windows with Visual Studio C++ Build Tools, the Windows SDK, and the Rust MSVC toolchain.",
    )
  }
  const interactive = isInteractive()
  log(`Node.js ${process.version}`)
  log(`pnpm ${capture("pnpm", ["--version"])}`)
  log(capture("cargo", ["--version"]))
  log(capture("vpk", ["--version"]))
  log(`Requires Visual Studio C++ Build Tools, Windows SDK, and: rustup target add ${TARGET}`)
  const signing = requiredSigning()
  if (signing) {
    log(`Code signing: ${signing.describe}`)
  } else {
    logError("WARNING: building unsigned packages. SmartScreen and antivirus will flag them.")
  }

  const config = readConfig()
  const packageName = `LitematicaPreview-${config.version}-x86_64`
  const portableRoot = path.join(ARTIFACTS_ROOT, "x86_64")
  const targetDir = path.resolve(process.env.CARGO_TARGET_DIR ?? path.join(ROOT, "target"))
  const releaseRoot = path.join(targetDir, TARGET, "release")
  const setupPath = path.join(ARTIFACTS_ROOT, `${packageName}-setup.exe`)
  const portableZip = path.join(ARTIFACTS_ROOT, `${packageName}-portable.zip`)
  assertPortableIsUnlocked(portableRoot)

  const env: NodeJS.ProcessEnv = {
    ...process.env,
    CARGO_TARGET_DIR: targetDir,
  }
  // Link the Visual C++ runtime statically so packages do not require a separate runtime installer.
  const rustflags = process.env.RUSTFLAGS ?? ""
  if (!rustflags.includes("target-feature=+crt-static")) {
    env.RUSTFLAGS = `${rustflags} -C target-feature=+crt-static`.trim()
  }

  stage = "Building the application"
  log(`\n[2/6] ${stage}`)
  for (const stale of [setupPath, portableZip]) {
    fs.rmSync(stale, { force: true })
  }
  fs.rmSync(VELOPACK_ROOT, { force: true, recursive: true })
  const tauri = await run(
    "pnpm",
    ["exec", "tauri", "build", "--target", TARGET, "--ci", "--no-bundle", "--", "--locked"],
    env,
  )
  if (tauri !== 0) {
    throw new Error(
      "Tauri release build failed. Check the compiler and bundler output in the log; confirm the MSVC C++ tools, Windows SDK, and Rust target are installed.",
    )
  }

  stage = "Staging the application for Velopack"
  log(`\n[3/6] ${stage}`)
  const executable = path.join(releaseRoot, "LitematicaPreview.exe")
  if (!fs.existsSync(executable)) {
    throw new Error(`The release executable is missing: ${executable}`)
  }
  fs.rmSync(portableRoot, { force: true, recursive: true })
  fs.mkdirSync(portableRoot, { recursive: true })
  fs.copyFileSync(executable, path.join(portableRoot, "LitematicaPreview.exe"))
  stageResources(config.bundle.resources, portableRoot)
  for (const file of REQUIRED_FILES) {
    requireFile(path.join(portableRoot, file), "Required package file")
  }
  for (const directory of REQUIRED_DIRECTORIES) {
    const required = path.join(portableRoot, directory)
    if (!fs.existsSync(required) || !fs.statSync(required).isDirectory()) {
      throw new Error(`Required package directory is missing: ${required}`)
    }
    if (
      !fs
        .readdirSync(required, { recursive: true, withFileTypes: true })
        .some((entry) => entry.isFile())
    ) {
      throw new Error(`Required package directory is empty: ${required}`)
    }
  }

  stage = "Packing the Velopack setup, portable ZIP, and update feed"
  log(`\n[4/6] ${stage}`)
  fs.mkdirSync(VELOPACK_ROOT, { recursive: true })
  seedPreviousReleases()
  const packed = await run(
    "vpk",
    [
      "pack",
      "--packId",
      PACK_ID,
      "--packVersion",
      config.version,
      "--packDir",
      portableRoot,
      "--mainExe",
      "LitematicaPreview.exe",
      "--packTitle",
      "Litematica Preview",
      "--packAuthors",
      "Arcadi4",
      "--icon",
      path.join(ROOT, "Assets", "app.ico"),
      "--channel",
      CHANNEL,
      "--framework",
      "webview2",
      "--outputDir",
      VELOPACK_ROOT,
      ...(signing?.args ?? []),
    ],
    env,
  )
  if (packed !== 0) {
    throw new Error(
      "Velopack packing failed. Confirm vpk is 1.2.161 and the .NET 10 SDK is installed.",
    )
  }

  stage = "Verifying the generated packages"
  log(`\n[5/6] ${stage}`)
  const setup = findPackaged(VELOPACK_ROOT, "-Setup.exe", "Velopack setup executable")
  const portable = findPackaged(VELOPACK_ROOT, "-Portable.zip", "Velopack portable archive")
  const feed = path.join(VELOPACK_ROOT, `releases.${CHANNEL}.json`)
  requireFile(feed, "Velopack release feed")
  const fullPackage = fs
    .readdirSync(VELOPACK_ROOT)
    .find((name) => name.includes(`-${config.version}-`) && name.endsWith("-full.nupkg"))
  if (!fullPackage) {
    throw new Error(`Velopack did not produce a full update package for ${config.version}.`)
  }
  if (signing) {
    verifySignature(setup)
  }
  fs.copyFileSync(setup, setupPath)
  fs.copyFileSync(portable, portableZip)

  stage = "Reporting the release artifacts"
  log(`\n[6/6] ${stage}`)
  for (const generated of [setupPath, portableZip]) {
    requireFile(generated, "Generated package")
  }
  log(`\nBuild completed in ${elapsed()}.`)
  for (const generated of [setupPath, portableZip]) {
    log(describePackage(generated))
  }
  log(`Velopack feed: ${VELOPACK_ROOT}`)
  log(`Upload the feed files to the R2 prefix litematica-preview/windows/.`)
  log(`Portable staging: ${portableRoot}`)
  log(`Build log: ${logPath}`)
  log("Install: run the setup executable; it installs per-user and updates automatically.")
  log(
    "File associations are not part of the one-click setup; the app prompts after the first launch.",
  )
  if (!signing) {
    logError(
      "WARNING: these packages are unsigned. Do not publish them; configure code signing first.",
    )
  }
  if (interactive) {
    try {
      if (/^y(es)?$/i.test((await prompt("Open the artifacts folder? [y/N] ")).trim())) {
        openArtifactsFolder()
      }
    } catch (cause) {
      log(
        `The build succeeded, but the artifacts folder could not be opened: ${(cause as Error).message}`,
      )
    }
  }
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
