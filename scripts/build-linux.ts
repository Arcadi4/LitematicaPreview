// Builds the Linux x64 release artifacts: a Debian package and an AppImage.
// Pass `--apt` to embed the signed apt repository source and public key in the
// Debian package. Run it through the package scripts: `pnpm run build:linux` or
// `pnpm run build:linux:apt`.

import { spawn, spawnSync, type ChildProcess } from "node:child_process"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"
import {
  APT_ARCHITECTURE,
  APT_KEYRING_PATH,
  APT_PACKAGE,
  APT_SOURCE_PATH,
  normalizeAptBaseUrl,
  renderAptSource,
} from "./apt"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const TARGET = "x86_64-unknown-linux-gnu"
const TAURI_ROOT = path.join(ROOT, "src-tauri")
const ARTIFACTS_ROOT = path.join(ROOT, "artifacts")
const LOGS_ROOT = path.join(ARTIFACTS_ROOT, "logs")
const APT_GENERATED_ROOT = path.join(TAURI_ROOT, "apt", "generated")
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/
const MISSING_TOOL: Record<string, string> = {
  pnpm: "pnpm is not on PATH. Install Node.js 24 and the pnpm version declared in package.json, then open a new terminal.",
  cargo:
    "cargo is not on PATH. Install Rust from https://rustup.rs, reopen the terminal, then run: rustup target add x86_64-unknown-linux-gnu.",
  gpg: "gpg is not on PATH. Install GnuPG (package: gnupg) to embed the apt repository key.",
  "dpkg-deb": "dpkg-deb is not on PATH. Install dpkg to verify the generated Debian package.",
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

interface AptMaterials {
  baseUrl: string
  source: string
  configPath: string
  keyringPath: string
  fingerprints: string[]
}

function environment(name: string): string | undefined {
  const value = process.env[name]?.trim()
  return value ? value : undefined
}

function requireEnvironment(name: string, describe: string): string {
  const value = environment(name)
  if (!value) {
    throw new Error(`${name} is not set. It provides ${describe}.`)
  }
  return value
}

function execute(tool: string, args: string[], input?: Buffer): void {
  const result = spawnSync(tool, args, { cwd: ROOT, input, encoding: "utf8" })
  if (result.error) {
    throw new Error(MISSING_TOOL[tool] ?? result.error.message)
  }
  if (result.status !== 0) {
    throw new Error(
      `${tool} ${args.join(" ")} failed: ${result.stderr.trim() || `exit code ${result.status}`}`,
    )
  }
}

function readAptPublicKey(): Buffer {
  const file = environment("LP_APT_PUBLIC_KEY_FILE")
  const inline = environment("LP_APT_PUBLIC_KEY")
  if (file && inline) {
    throw new Error("Set only one of LP_APT_PUBLIC_KEY_FILE and LP_APT_PUBLIC_KEY.")
  }
  if (file) {
    requireFile(file, "Apt repository public key")
    return fs.readFileSync(file)
  }
  if (inline) {
    return Buffer.from(inline.endsWith("\n") ? inline : `${inline}\n`)
  }
  throw new Error(
    "The apt repository public key is missing. Set LP_APT_PUBLIC_KEY_FILE or LP_APT_PUBLIC_KEY.",
  )
}

function writeAptKeyring(contents: Buffer, target: string): void {
  if (contents.length === 0) {
    throw new Error("The apt repository public key is empty.")
  }
  const text = contents.toString("utf8")
  if (text.includes("-----BEGIN PGP PRIVATE KEY BLOCK-----")) {
    throw new Error("The apt repository key must be a public key, not a private key.")
  }
  if (text.includes("-----BEGIN PGP PUBLIC KEY BLOCK-----")) {
    execute("gpg", ["--batch", "--yes", "--dearmor", "--output", target], contents)
    return
  }
  fs.writeFileSync(target, contents)
}

function inspectPublicKey(keyringPath: string): string[] {
  const colon = capture("gpg", ["--batch", "--with-colons", "--show-keys", keyringPath])
  const fingerprints: string[] = []
  let expectsFingerprint = false
  for (const line of colon.split("\n")) {
    const fields = line.split(":")
    if (fields[0] === "sec" || fields[0] === "ssb") {
      throw new Error("The apt repository key must be a public key, not a private key.")
    }
    if (fields[0] === "pub") {
      const validity = fields[1]
      if (validity === "e" || validity === "r" || validity === "i" || validity === "d") {
        throw new Error(`The apt repository public key is not usable (GnuPG status: ${validity}).`)
      }
      expectsFingerprint = true
    } else if (fields[0] === "fpr" && expectsFingerprint) {
      const fingerprint = fields[9]
      if (!fingerprint) {
        throw new Error("GnuPG did not report a fingerprint for the apt repository public key.")
      }
      fingerprints.push(fingerprint)
      expectsFingerprint = false
    } else if (fields[0] === "sub" || fields[0] === "uid") {
      expectsFingerprint = false
    }
  }
  if (fingerprints.length === 0) {
    throw new Error("The apt repository public key contains no key.")
  }
  return fingerprints
}

function readLinuxDebFiles(): Record<string, string> {
  const parsed: unknown = JSON.parse(
    fs.readFileSync(path.join(TAURI_ROOT, "tauri.linux.conf.json"), "utf8"),
  )
  const files: Record<string, string> = {}
  if (typeof parsed !== "object" || parsed === null) {
    return files
  }
  const bundle = (parsed as Record<string, unknown>).bundle
  if (typeof bundle !== "object" || bundle === null) {
    return files
  }
  const linux = (bundle as Record<string, unknown>).linux
  if (typeof linux !== "object" || linux === null) {
    return files
  }
  const deb = (linux as Record<string, unknown>).deb
  if (typeof deb !== "object" || deb === null) {
    return files
  }
  const declared = (deb as Record<string, unknown>).files
  if (typeof declared !== "object" || declared === null || Array.isArray(declared)) {
    return files
  }
  for (const [target, source] of Object.entries(declared)) {
    if (typeof source !== "string" || source.trim() === "") {
      throw new Error(`tauri.linux.conf.json maps ${target} to an invalid source path.`)
    }
    files[target] = source
  }
  return files
}

function prepareAptFiles(): AptMaterials {
  const baseUrl = normalizeAptBaseUrl(
    requireEnvironment("LP_APT_BASE_URL", "the public apt repository URL"),
  )
  fs.rmSync(APT_GENERATED_ROOT, { force: true, recursive: true })
  fs.mkdirSync(APT_GENERATED_ROOT, { recursive: true })

  const keyringPath = path.join(APT_GENERATED_ROOT, "litematica-preview-archive-keyring.gpg")
  writeAptKeyring(readAptPublicKey(), keyringPath)
  fs.chmodSync(keyringPath, 0o644)
  const fingerprints = inspectPublicKey(keyringPath)

  const source = renderAptSource(baseUrl)
  const sourcePath = path.join(APT_GENERATED_ROOT, "litematica-preview.list")
  fs.writeFileSync(sourcePath, source, { mode: 0o644 })
  fs.chmodSync(sourcePath, 0o644)

  const files = readLinuxDebFiles()
  for (const target of [APT_SOURCE_PATH, APT_KEYRING_PATH]) {
    if (target in files) {
      throw new Error(`tauri.linux.conf.json already declares bundle file ${target}.`)
    }
  }
  const bundle = {
    bundle: {
      linux: {
        deb: {
          files: {
            ...files,
            [APT_SOURCE_PATH]: "apt/generated/litematica-preview.list",
            [APT_KEYRING_PATH]: "apt/generated/litematica-preview-archive-keyring.gpg",
          },
        },
      },
    },
  }
  const configPath = path.join(TAURI_ROOT, "apt-bundle.json")
  fs.writeFileSync(configPath, `${JSON.stringify(bundle, null, 2)}\n`)
  return { baseUrl, source, configPath, keyringPath, fingerprints }
}

function verifyAptPackage(debPath: string, apt: AptMaterials): void {
  const control = ["Package", "Architecture"].map((field) =>
    capture("dpkg-deb", ["-f", debPath, field]).trim(),
  )
  if (control[0] !== APT_PACKAGE) {
    throw new Error(`Expected Debian package ${APT_PACKAGE}; found ${control[0] ?? "none"}.`)
  }
  if (control[1] !== APT_ARCHITECTURE) {
    throw new Error(
      `Expected Debian architecture ${APT_ARCHITECTURE}; found ${control[1] ?? "none"}.`,
    )
  }

  const extractRoot = fs.mkdtempSync(path.join(os.tmpdir(), "lp-apt-deb-"))
  try {
    execute("dpkg-deb", ["-x", debPath, extractRoot])
    const sourcePath = path.join(extractRoot, `.${APT_SOURCE_PATH}`)
    const keyringPath = path.join(extractRoot, `.${APT_KEYRING_PATH}`)
    for (const embedded of [sourcePath, keyringPath]) {
      if (!fs.existsSync(embedded)) {
        throw new Error(
          `The Debian package does not contain ${embedded.slice(extractRoot.length)}.`,
        )
      }
    }
    if (fs.readFileSync(sourcePath, "utf8") !== apt.source) {
      throw new Error(
        `The Debian package contains a different apt source than ${apt.source.trim()}.`,
      )
    }
    if (!fs.readFileSync(keyringPath).equals(fs.readFileSync(apt.keyringPath))) {
      throw new Error("The Debian package contains a different apt repository keyring.")
    }
    for (const embedded of [sourcePath, keyringPath]) {
      if ((fs.statSync(embedded).mode & 0o444) !== 0o444) {
        throw new Error(
          `The Debian package makes ${embedded.slice(extractRoot.length)} unreadable to apt.`,
        )
      }
    }
  } finally {
    fs.rmSync(extractRoot, { force: true, recursive: true })
  }
}

async function main(): Promise<void> {
  const buildArguments = process.argv.slice(2).filter((argument) => argument !== "--")
  const unsupported = buildArguments.filter((argument) => argument !== "--apt")
  if (unsupported.length) {
    throw new Error(
      `Unsupported argument: ${unsupported[0]}. Use --apt to embed the apt repository.`,
    )
  }
  const aptEnabled = buildArguments.includes("--apt")

  if (process.platform !== "linux" || process.arch !== "x64") {
    throw new Error(
      `Linux x64 packages can only be produced on Linux x64, but this host is ${process.platform} ${process.arch}. ` +
        "Use Ubuntu 22.04 or another Debian-based distribution with GTK 3, WebKitGTK 4.1, and the Rust target.",
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
  let apt: AptMaterials | undefined
  if (aptEnabled) {
    stage = "Preparing the apt repository files"
    apt = prepareAptFiles()
    log(`Apt repository: ${apt.baseUrl}`)
    log(`Apt signing key: ${apt.fingerprints.join(", ")}`)
    stage = "Checking prerequisites"
  }
  const packageName = `LitematicaPreview-${config.version}-x86_64`
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
  const tauriArgs = [
    "exec",
    "tauri",
    "build",
    "--target",
    TARGET,
    "--ci",
    "--bundles",
    "deb,appimage",
  ]
  if (apt) {
    tauriArgs.push("--config", apt.configPath)
  }
  tauriArgs.push("--", "--locked")
  const tauri = await run("pnpm", tauriArgs, { ...process.env, CARGO_TARGET_DIR: targetDir })
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
  if (apt) {
    verifyAptPackage(debPath, apt)
    log(`Verified the embedded apt source and keyring in ${path.basename(debPath)}`)
  }
  log(`\nBuild completed in ${elapsed()}.`)
  for (const generated of [debPath, appImagePath]) {
    log(describePackage(generated))
  }
  log(`Build log: ${logPath}`)
  log(`Install the Debian package: sudo apt install ./${path.basename(debPath)}`)
  if (apt) {
    log(`The Debian package enrolls ${apt.baseUrl}; later releases arrive through apt upgrade.`)
  }
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
