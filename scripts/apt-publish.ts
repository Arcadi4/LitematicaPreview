// Publishes a Linux Debian package to the signed apt repository on Cloudflare R2.
// Run it through the package script: `pnpm run apt:publish -- --deb <package.deb>`.

import { spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

import {
  APT_ARCHITECTURE,
  APT_COMPONENT,
  APT_DISTRIBUTION,
  APT_KEYRING_PATH,
  APT_PACKAGE,
  APT_SOURCE_PATH,
  normalizeAptBaseUrl,
  renderAptSource,
} from "./apt"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const DEFAULT_REPOSITORY = path.join(ROOT, "target", "apt-repo")
const MISSING_TOOL: Record<string, string> = {
  gpg: "gpg is not on PATH. Install GnuPG (package: gnupg).",
  gpgv: "gpgv is not on PATH. Install GnuPG (package: gnupg).",
  reprepro: "reprepro is not on PATH. Install reprepro on the publishing host.",
  "dpkg-deb": "dpkg-deb is not on PATH. Install dpkg.",
  cf: "cf is not on PATH. Install the Cloudflare CLI: npm install --global cf.",
}

interface Options {
  debs: string[]
  repo: string
  allowPrerelease: boolean
  dryRun: boolean
}

interface DebPackage {
  path: string
  name: string
  version: string
  architecture: string
  source: string
  keyring: Buffer
}

interface GpgIdentity {
  fingerprint: string
  env: NodeJS.ProcessEnv
}

function log(message: string): void {
  process.stdout.write(`${message}\n`)
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

function runTool(tool: string, args: string[], env: NodeJS.ProcessEnv = process.env): void {
  const result = spawnSync(tool, args, { cwd: ROOT, env, encoding: "utf8" })
  if (result.error) {
    throw new Error(MISSING_TOOL[tool] ?? result.error.message)
  }
  if (result.status !== 0) {
    throw new Error(
      `${tool} ${args.join(" ")} failed: ${result.stderr.trim() || `exit code ${result.status}`}`,
    )
  }
}

function captureTool(tool: string, args: string[], env: NodeJS.ProcessEnv = process.env): string {
  const result = spawnSync(tool, args, { cwd: ROOT, env, encoding: "utf8" })
  if (result.error) {
    throw new Error(MISSING_TOOL[tool] ?? result.error.message)
  }
  if (result.status !== 0) {
    throw new Error(
      `${tool} ${args.join(" ")} failed: ${result.stderr.trim() || `exit code ${result.status}`}`,
    )
  }
  return result.stdout
}

function requireFile(target: string, describe: string): void {
  let stats: fs.Stats
  try {
    stats = fs.statSync(target)
  } catch {
    throw new Error(`${describe} is missing: ${target}`)
  }
  if (!stats.isFile() || stats.size === 0) {
    throw new Error(`${describe} is not a readable file: ${target}`)
  }
}

function printUsage(): void {
  process.stdout.write(
    [
      "Publishes a Debian package to the signed apt repository on Cloudflare R2.",
      "",
      "Usage: pnpm run apt:publish -- --deb <package.deb> [options]",
      "",
      "Options:",
      "  --deb <path>          Debian package to publish (repeatable, required)",
      `  --repo <path>         Local reprepro repository (default: ${DEFAULT_REPOSITORY})`,
      "  --allow-prerelease    Publish a version that contains a prerelease suffix",
      "  --dry-run             Build and sign the repository without uploading",
      "  --help                Show this message",
      "",
      "Required environment:",
      "  LP_APT_BASE_URL            Public https URL of the repository",
      "  LP_APT_GPG_PRIVATE_KEY     Armored private signing key (or LP_APT_GPG_PRIVATE_KEY_FILE)",
      "  LP_APT_R2_BUCKET           R2 bucket that serves the repository",
      "  CLOUDFLARE_API_TOKEN       API token with R2 write access",
      "  CLOUDFLARE_ACCOUNT_ID      Account that owns the bucket",
      "",
      "Optional environment:",
      "  LP_APT_GPG_PASSPHRASE      Passphrase for a protected signing key",
      "  LP_APT_GPG_FINGERPRINT     Require a specific signing key fingerprint",
      "  LP_APT_R2_JURISDICTION     R2 jurisdiction: default, eu, fedramp, or us",
      "",
    ].join("\n"),
  )
}

function parseOptions(argv: string[]): Options {
  const options: Options = {
    debs: [],
    repo: DEFAULT_REPOSITORY,
    allowPrerelease: false,
    dryRun: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === "--") {
      continue
    }
    if (argument === "--deb") {
      const value = argv[index + 1]
      if (!value) {
        throw new Error("--deb requires a path.")
      }
      options.debs.push(path.resolve(ROOT, value))
      index += 1
    } else if (argument === "--repo") {
      const value = argv[index + 1]
      if (!value) {
        throw new Error("--repo requires a path.")
      }
      options.repo = path.resolve(ROOT, value)
      index += 1
    } else if (argument === "--allow-prerelease") {
      options.allowPrerelease = true
    } else if (argument === "--dry-run") {
      options.dryRun = true
    } else if (argument === "--help") {
      printUsage()
      process.exit(0)
    } else {
      throw new Error(`Unsupported argument: ${argument}. Run with --help for usage.`)
    }
  }
  if (options.debs.length === 0) {
    throw new Error("Pass at least one --deb package to publish.")
  }
  return options
}

function readPrivateKey(): Buffer {
  const file = environment("LP_APT_GPG_PRIVATE_KEY_FILE")
  const inline = environment("LP_APT_GPG_PRIVATE_KEY")
  if (file && inline) {
    throw new Error("Set only one of LP_APT_GPG_PRIVATE_KEY_FILE and LP_APT_GPG_PRIVATE_KEY.")
  }
  if (file) {
    requireFile(file, "Apt signing key")
    return fs.readFileSync(file)
  }
  if (inline) {
    return Buffer.from(inline.endsWith("\n") ? inline : `${inline}\n`)
  }
  throw new Error(
    "The apt signing key is missing. Set LP_APT_GPG_PRIVATE_KEY or LP_APT_GPG_PRIVATE_KEY_FILE. " +
      "The publisher never creates or generates keys.",
  )
}

function primaryFingerprints(colon: string, record: string): string[] {
  const fingerprints: string[] = []
  let expectsFingerprint = false
  for (const line of colon.split("\n")) {
    const fields = line.split(":")
    if (fields[0] === record) {
      expectsFingerprint = true
    } else if (fields[0] === "fpr" && expectsFingerprint) {
      const fingerprint = fields[9]
      if (fingerprint) {
        fingerprints.push(fingerprint)
      }
      expectsFingerprint = false
    } else if (fields[0] === "sub" || fields[0] === "ssb" || fields[0] === "uid") {
      expectsFingerprint = false
    }
  }
  return fingerprints
}

function presetPassphrase(home: string, env: NodeJS.ProcessEnv, passphrase: string): void {
  fs.writeFileSync(path.join(home, "gpg-agent.conf"), "allow-preset-passphrase\n")
  spawnSync("gpgconf", ["--kill", "gpg-agent"], { env, encoding: "utf8" })
  runTool("gpgconf", ["--launch", "gpg-agent"], env)
  const keygrips = captureTool(
    "gpg",
    ["--batch", "--with-colons", "--with-keygrip", "--list-secret-keys"],
    env,
  )
    .split("\n")
    .filter((line) => line.startsWith("grp:"))
    .map((line) => line.split(":")[9] ?? "")
    .filter((keygrip) => keygrip !== "")
  const preset = locatePresetCommand(env)
  for (const keygrip of keygrips) {
    const result = spawnSync(preset, ["--preset", keygrip], {
      cwd: ROOT,
      env,
      encoding: "utf8",
      input: `${passphrase}\n`,
    })
    if (result.error || result.status !== 0) {
      throw new Error(
        `gpg-preset-passphrase failed for the signing key: ${result.error?.message ?? result.stderr.trim()}`,
      )
    }
  }
}

function locatePresetCommand(env: NodeJS.ProcessEnv): string {
  const which = spawnSync("which", ["gpg-preset-passphrase"], { env, encoding: "utf8" })
  const found = which.status === 0 ? which.stdout.trim() : ""
  if (found) {
    return found
  }
  const directories = captureTool("gpgconf", ["--list-dirs"], env)
  const line = directories.split("\n").find((entry) => entry.startsWith("libexecdir:"))
  if (line) {
    const candidate = path.join(line.slice("libexecdir:".length).trim(), "gpg-preset-passphrase")
    if (fs.existsSync(candidate)) {
      return candidate
    }
  }
  throw new Error("gpg-preset-passphrase is required for LP_APT_GPG_PASSPHRASE but was not found.")
}

function prepareSigningKey(workRoot: string): GpgIdentity {
  const home = path.join(workRoot, "gnupg")
  fs.mkdirSync(home, { recursive: true, mode: 0o700 })
  const env: NodeJS.ProcessEnv = { ...process.env, GNUPGHOME: home, LC_ALL: "C" }

  const imported = spawnSync("gpg", ["--batch", "--yes", "--import"], {
    cwd: ROOT,
    env,
    encoding: "utf8",
    input: readPrivateKey(),
  })
  if (imported.error) {
    throw new Error(MISSING_TOOL.gpg ?? imported.error.message)
  }
  if (imported.status !== 0) {
    throw new Error(`gpg could not import the signing key: ${imported.stderr.trim()}`)
  }

  const listed = captureTool("gpg", ["--batch", "--with-colons", "--list-secret-keys"], env)
  const fingerprints = primaryFingerprints(listed, "sec")
  if (fingerprints.length === 0) {
    throw new Error(
      "The configured key contains no secret key; the publisher never generates keys.",
    )
  }
  const expected = environment("LP_APT_GPG_FINGERPRINT")?.toUpperCase()
  const fingerprint = expected
    ? fingerprints.find((candidate) => candidate.toUpperCase() === expected)
    : fingerprints[0]
  if (!fingerprint) {
    throw new Error(`The signing key ${expected} is not present in the configured private key.`)
  }

  const passphrase = environment("LP_APT_GPG_PASSPHRASE")
  if (passphrase) {
    presetPassphrase(home, env, passphrase)
  }
  return { fingerprint, env }
}

function readEmbedded(root: string, absolutePath: string): Buffer {
  const target = path.join(root, `.${absolutePath}`)
  if (!fs.existsSync(target)) {
    throw new Error(
      `The Debian package does not embed ${absolutePath}. Build it with scripts/build-linux.ts --apt.`,
    )
  }
  return fs.readFileSync(target)
}

function inspectDeb(debPath: string, workRoot: string, index: number): DebPackage {
  requireFile(debPath, "Debian package")
  const control = ["Package", "Version", "Architecture"].map((field) =>
    captureTool("dpkg-deb", ["-f", debPath, field]).trim(),
  )
  const [name, version, architecture] = control
  if (!name || !version || !architecture) {
    throw new Error(`Could not read the Debian control fields from ${debPath}.`)
  }
  const extractRoot = path.join(workRoot, `deb-${index}`)
  fs.mkdirSync(extractRoot)
  runTool("dpkg-deb", ["-x", debPath, extractRoot])
  return {
    path: debPath,
    name,
    version,
    architecture,
    source: readEmbedded(extractRoot, APT_SOURCE_PATH).toString("utf8"),
    keyring: readEmbedded(extractRoot, APT_KEYRING_PATH),
  }
}

function validatePackages(packages: DebPackage[], baseUrl: string, allowPrerelease: boolean): void {
  const expectedSource = renderAptSource(baseUrl)
  for (const deb of packages) {
    if (deb.name !== APT_PACKAGE) {
      throw new Error(
        `${path.basename(deb.path)} is package ${deb.name}; the apt repository publishes ${APT_PACKAGE}.`,
      )
    }
    if (deb.architecture !== APT_ARCHITECTURE) {
      throw new Error(
        `${path.basename(deb.path)} targets ${deb.architecture}; the apt repository serves ${APT_ARCHITECTURE}.`,
      )
    }
    if (deb.version.includes("-") && !allowPrerelease) {
      throw new Error(
        `${deb.name} ${deb.version} is a prerelease. Pass --allow-prerelease to publish it to apt.`,
      )
    }
    if (deb.source !== expectedSource) {
      throw new Error(
        `${path.basename(deb.path)} does not embed ${expectedSource.trim()}. Rebuild it with --apt and the matching LP_APT_BASE_URL.`,
      )
    }
  }
}

function keyFingerprints(keyringPath: string, env: NodeJS.ProcessEnv): string[] {
  const colon = captureTool("gpg", ["--batch", "--with-colons", "--show-keys", keyringPath], env)
  for (const line of colon.split("\n")) {
    const record = line.split(":")[0]
    if (record === "sec" || record === "ssb") {
      throw new Error(
        "The Debian package embeds private key material. Rebuild it with a public key.",
      )
    }
  }
  return primaryFingerprints(colon, "pub")
}

function validateEmbeddedKeyring(
  packages: DebPackage[],
  identity: GpgIdentity,
  workRoot: string,
): string {
  const reference = packages[0].keyring
  for (const deb of packages) {
    if (!deb.keyring.equals(reference)) {
      throw new Error(`The Debian packages embed different apt repository keyrings.`)
    }
  }
  const keyringPath = path.join(workRoot, "embedded-keyring.gpg")
  fs.writeFileSync(keyringPath, reference)
  const fingerprints = keyFingerprints(keyringPath, identity.env)
  const trusted = fingerprints.some(
    (fingerprint) => fingerprint.toUpperCase() === identity.fingerprint.toUpperCase(),
  )
  if (!trusted) {
    throw new Error(
      `The Debian package embeds ${fingerprints.join(", ") || "no key"}, which does not match signing key ${identity.fingerprint}.`,
    )
  }
  return keyringPath
}

function createRepository(repoRoot: string, fingerprint: string): void {
  const resolvedRoot = fs.realpathSync(ROOT)
  const resolvedRepo = fs.existsSync(repoRoot) ? fs.realpathSync(repoRoot) : repoRoot
  if (
    resolvedRepo === path.parse(resolvedRepo).root ||
    resolvedRepo === resolvedRoot ||
    resolvedRoot.startsWith(`${resolvedRepo}${path.sep}`)
  ) {
    throw new Error(`Refusing to replace repository directory ${repoRoot}.`)
  }
  if (fs.existsSync(repoRoot)) {
    const marker = path.join(repoRoot, ".litematica-apt-repository")
    if (!fs.existsSync(marker)) {
      throw new Error(`Refusing to replace an unrecognized directory: ${repoRoot}.`)
    }
    fs.rmSync(repoRoot, { force: true, recursive: true })
  }
  fs.mkdirSync(path.join(repoRoot, "conf"), { recursive: true })
  fs.writeFileSync(path.join(repoRoot, ".litematica-apt-repository"), "")
  fs.writeFileSync(
    path.join(repoRoot, "conf", "distributions"),
    [
      "Origin: Litematica Preview",
      "Label: Litematica Preview",
      `Suite: ${APT_DISTRIBUTION}`,
      `Codename: ${APT_DISTRIBUTION}`,
      `Architectures: ${APT_ARCHITECTURE}`,
      `Components: ${APT_COMPONENT}`,
      "Description: Litematica Preview Debian repository",
      `SignWith: ${fingerprint}`,
      "DebIndices: Packages Release . .gz .xz",
      "",
    ].join("\n"),
  )
  fs.writeFileSync(path.join(repoRoot, "conf", "options"), "verbose\n")
}

function reprepro(repoRoot: string, args: string[], env: NodeJS.ProcessEnv): void {
  runTool("reprepro", ["--basedir", repoRoot, ...args], env)
}

function buildRepository(repoRoot: string, packages: DebPackage[], identity: GpgIdentity): void {
  createRepository(repoRoot, identity.fingerprint)
  for (const deb of packages) {
    reprepro(
      repoRoot,
      [
        "-C",
        APT_COMPONENT,
        "-S",
        "utils",
        "-P",
        "optional",
        "includedeb",
        APT_DISTRIBUTION,
        deb.path,
      ],
      identity.env,
    )
    log(`Included ${deb.name} ${deb.version}`)
  }
  reprepro(repoRoot, ["export", APT_DISTRIBUTION], identity.env)
  const distribution = path.join(repoRoot, "dists", APT_DISTRIBUTION)
  const indexDir = path.join(distribution, APT_COMPONENT, `binary-${APT_ARCHITECTURE}`)
  const byHash = path.join(indexDir, "by-hash", "SHA256")
  fs.mkdirSync(byHash, { recursive: true })
  for (const name of ["Packages", "Packages.gz", "Packages.xz"]) {
    const index = path.join(indexDir, name)
    requireFile(index, `Generated repository index ${name}`)
    fs.copyFileSync(index, path.join(byHash, sha256(fs.readFileSync(index))))
  }
  const release = path.join(distribution, "Release")
  fs.appendFileSync(release, "Acquire-By-Hash: yes\n")
  const gpg = ["--batch", "--yes", "--local-user", identity.fingerprint]
  runTool(
    "gpg",
    [
      ...gpg,
      "--armor",
      "--detach-sign",
      "--output",
      path.join(distribution, "Release.gpg"),
      release,
    ],
    identity.env,
  )
  runTool(
    "gpg",
    [...gpg, "--clearsign", "--output", path.join(distribution, "InRelease"), release],
    identity.env,
  )
}

function verifyRepository(repoRoot: string, keyringPath: string, identity: GpgIdentity): void {
  const distributionRoot = path.join(repoRoot, "dists", APT_DISTRIBUTION)
  for (const required of [
    "InRelease",
    "Release",
    "Release.gpg",
    path.join(APT_COMPONENT, `binary-${APT_ARCHITECTURE}`, "Packages"),
  ]) {
    requireFile(path.join(distributionRoot, required), `Generated repository file ${required}`)
  }
  runTool(
    "gpgv",
    ["--keyring", keyringPath, path.join(distributionRoot, "InRelease")],
    identity.env,
  )
  runTool(
    "gpgv",
    [
      "--keyring",
      keyringPath,
      path.join(distributionRoot, "Release.gpg"),
      path.join(distributionRoot, "Release"),
    ],
    identity.env,
  )
}

function parseControl(stanza: string): Map<string, string> {
  const fields = new Map<string, string>()
  for (const line of stanza.split("\n")) {
    if (line.startsWith(" ") || line.startsWith("\t")) {
      continue
    }
    const separator = line.indexOf(":")
    if (separator === -1) {
      continue
    }
    fields.set(line.slice(0, separator).trim(), line.slice(separator + 1).trim())
  }
  return fields
}

function validateIndex(repoRoot: string, packages: DebPackage[]): void {
  const packagesPath = path.join(
    repoRoot,
    "dists",
    APT_DISTRIBUTION,
    APT_COMPONENT,
    `binary-${APT_ARCHITECTURE}`,
    "Packages",
  )
  requireFile(packagesPath, "Generated Packages index")
  const expected = new Map(packages.map((deb) => [deb.name, deb.version]))
  const seen = new Map<string, string>()
  for (const stanza of fs.readFileSync(packagesPath, "utf8").split(/\n\s*\n/)) {
    const fields = parseControl(stanza.trim())
    const name = fields.get("Package")
    if (!name) {
      continue
    }
    if (!expected.has(name)) {
      throw new Error(`The repository index contains unexpected package ${name}.`)
    }
    if (seen.has(name)) {
      throw new Error(`The repository index contains more than one version of ${name}.`)
    }
    const version = fields.get("Version") ?? ""
    if (version !== expected.get(name)) {
      throw new Error(
        `The repository index lists ${name} ${version}; expected ${expected.get(name)}.`,
      )
    }
    if (fields.get("Architecture") !== APT_ARCHITECTURE) {
      throw new Error(`The repository index lists ${name} for an unexpected architecture.`)
    }
    const filename = fields.get("Filename")
    if (!filename || !fs.existsSync(path.join(repoRoot, filename))) {
      throw new Error(`The repository index points to a missing pool file for ${name}.`)
    }
    seen.set(name, version)
  }
  for (const name of expected.keys()) {
    if (!seen.has(name)) {
      throw new Error(`The repository index does not list ${name}.`)
    }
  }
}

function walk(directory: string, visit: (file: string) => void): void {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const absolute = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      walk(absolute, visit)
    } else if (entry.isFile()) {
      visit(absolute)
    }
  }
}

function repositoryKeys(repoRoot: string): string[] {
  const keys: string[] = []
  for (const root of ["pool", "dists"]) {
    const directory = path.join(repoRoot, root)
    if (fs.existsSync(directory)) {
      walk(directory, (file) => {
        keys.push(path.relative(repoRoot, file).split(path.sep).join("/"))
      })
    }
  }
  return keys.sort()
}

function publicationOrder(keys: string[]): string[] {
  const inRelease = `dists/${APT_DISTRIBUTION}/InRelease`
  if (!keys.includes(inRelease)) {
    throw new Error(`The repository does not contain ${inRelease}.`)
  }
  const release = new Set([
    `dists/${APT_DISTRIBUTION}/Release`,
    `dists/${APT_DISTRIBUTION}/Release.gpg`,
  ])
  const componentPrefix = `dists/${APT_DISTRIBUTION}/${APT_COMPONENT}/`
  const pool = keys.filter((key) => key.startsWith("pool/")).sort()
  const metadata = keys.filter((key) => key.startsWith(componentPrefix)).sort()
  const releaseKeys = keys.filter((key) => release.has(key)).sort()
  const remaining = keys
    .filter(
      (key) =>
        !key.startsWith("pool/") &&
        !key.startsWith(componentPrefix) &&
        !release.has(key) &&
        key !== inRelease,
    )
    .sort()
  return [...pool, ...metadata, ...releaseKeys, ...remaining, inRelease]
}

function contentType(key: string): string {
  if (key.endsWith(".deb")) {
    return "application/vnd.debian.binary-package"
  }
  if (key.endsWith(".gz")) {
    return "application/gzip"
  }
  if (key.endsWith(".xz")) {
    return "application/x-xz"
  }
  if (key.endsWith(".gpg")) {
    return "application/pgp-signature"
  }
  return "text/plain; charset=utf-8"
}

function uploadObject(repoRoot: string, key: string, bucket: string, dryRun: boolean): void {
  if (dryRun) {
    log(`PUT ${key}`)
    return
  }
  const args = [
    "r2",
    "objects",
    "put",
    key,
    "--bucket-name",
    bucket,
    "--file",
    path.join(repoRoot, key),
    "--content-type",
    contentType(key),
    "--quiet",
  ]
  const jurisdiction = environment("LP_APT_R2_JURISDICTION")
  if (jurisdiction && jurisdiction !== "default") {
    args.push("--cf-r2-jurisdiction", jurisdiction)
  }
  runTool("cf", args, { ...process.env, CF_SEND_TELEMETRY: "false" })
  log(`PUT ${key}`)
}

function sha256(contents: Buffer): string {
  return createHash("sha256").update(contents).digest("hex")
}

async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2))
  const baseUrl = normalizeAptBaseUrl(
    requireEnvironment("LP_APT_BASE_URL", "the public apt repository URL"),
  )
  const bucket = options.dryRun
    ? (environment("LP_APT_R2_BUCKET") ?? "dry-run")
    : requireEnvironment("LP_APT_R2_BUCKET", "the R2 bucket name")
  if (!options.dryRun) {
    requireEnvironment("CLOUDFLARE_API_TOKEN", "the Cloudflare API token with R2 write access")
    requireEnvironment("CLOUDFLARE_ACCOUNT_ID", "the Cloudflare account that owns the bucket")
  }

  const workRoot = fs.mkdtempSync(path.join(os.tmpdir(), "lp-apt-publish-"))
  let identity: GpgIdentity | undefined
  try {
    const packages = options.debs.map((deb, index) => inspectDeb(deb, workRoot, index))
    validatePackages(packages, baseUrl, options.allowPrerelease)

    identity = prepareSigningKey(workRoot)
    log(`Signing key: ${identity.fingerprint}`)
    const keyringPath = validateEmbeddedKeyring(packages, identity, workRoot)

    buildRepository(options.repo, packages, identity)
    verifyRepository(options.repo, keyringPath, identity)
    validateIndex(options.repo, packages)

    const keys = publicationOrder(repositoryKeys(options.repo))
    log(`Publishing ${keys.length} objects to ${bucket}:`)
    for (const key of keys) {
      uploadObject(options.repo, key, bucket, options.dryRun)
    }

    // No fetch-back through the custom domain: the zone's WAF and bot products may refuse
    // this client while still serving the object to APT clients, and each PUT already
    // confirmed the write.
    log(
      options.dryRun
        ? `Dry run complete. Local repository: ${options.repo}`
        : `Published ${baseUrl} from ${options.repo}`,
    )
  } finally {
    if (identity) {
      spawnSync("gpgconf", ["--kill", "gpg-agent"], { env: identity.env })
    }
    fs.rmSync(workRoot, { force: true, recursive: true })
  }
}

try {
  await main()
} catch (cause) {
  console.error(`Error: ${(cause as Error).message}`)
  process.exitCode = 1
}
