#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT_DIR = path.resolve(__dirname, "..");

interface VersionLocation {
  id: string;
  file: string;
  read: (filePath: string) => string | null;
  write: (filePath: string, newVersion: string) => void;
}

interface LocationResult extends VersionLocation {
  version: string | null;
  error: string | null;
}

interface BumpOptions {
  dryRun?: boolean;
}

interface ParsedSemver {
  raw: string;
  major: number;
  minor: number;
  patch: number;
  prerelease: string | null;
  build: string | null;
  normalized: string;
}

const LOCATIONS: VersionLocation[] = [
  {
    id: "App/package.json",
    file: path.join(ROOT_DIR, "App/package.json"),
    read(file: string): string | null {
      const json = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: string };
      return json.version ?? null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(^  "version"\s*:\s*)"[^"]+"/m,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "App/src-tauri/tauri.conf.json",
    file: path.join(ROOT_DIR, "App/src-tauri/tauri.conf.json"),
    read(file: string): string | null {
      const json = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: string };
      return json.version ?? null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(^  "version"\s*:\s*)"[^"]+"/m,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "App/src-tauri/Cargo.toml",
    file: path.join(ROOT_DIR, "App/src-tauri/Cargo.toml"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8");
      const match = content.match(
        /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m
      );
      return match ? match[1] : null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(^\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "Native/Cargo.toml",
    file: path.join(ROOT_DIR, "Native/Cargo.toml"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8");
      const match = content.match(
        /^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m
      );
      return match ? match[1] : null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(^\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "App/src-tauri/Cargo.lock (litematica-preview)",
    file: path.join(ROOT_DIR, "App/src-tauri/Cargo.lock"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8");
      const match = content.match(
        /\[\[package\]\]\s+name\s*=\s*"litematica-preview"\s+version\s*=\s*"([^"]+)"/m
      );
      return match ? match[1] : null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(\[\[package\]\]\s+name\s*=\s*"litematica-preview"\s+version\s*=\s*)"[^"]+"/g,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update litematica-preview version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "App/src-tauri/Cargo.lock (litematica-preview-native)",
    file: path.join(ROOT_DIR, "App/src-tauri/Cargo.lock"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8");
      const match = content.match(
        /\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*"([^"]+)"/m
      );
      return match ? match[1] : null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*)"[^"]+"/g,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update litematica-preview-native version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
  {
    id: "Native/Cargo.lock (litematica-preview-native)",
    file: path.join(ROOT_DIR, "Native/Cargo.lock"),
    read(file: string): string | null {
      const content = fs.readFileSync(file, "utf8");
      const match = content.match(
        /\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*"([^"]+)"/m
      );
      return match ? match[1] : null;
    },
    write(file: string, newVersion: string): void {
      const raw = fs.readFileSync(file, "utf8");
      const updated = raw.replace(
        /(\[\[package\]\]\s+name\s*=\s*"litematica-preview-native"\s+version\s*=\s*)"[^"]+"/g,
        `$1"${newVersion}"`
      );
      if (raw === updated && !raw.includes(`"${newVersion}"`)) {
        throw new Error(`Failed to update litematica-preview-native version in ${file}`);
      }
      fs.writeFileSync(file, updated, "utf8");
    },
  },
];

const SEMVER_REGEX =
  /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/;

function parseSemver(str: string): ParsedSemver | null {
  const match = String(str).trim().match(SEMVER_REGEX);
  if (!match) return null;
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
  };
}

function calculateBump(currentVersion: string, bumpType: string): string | null {
  const parsed = parseSemver(currentVersion);
  if (!parsed) {
    throw new Error(`Cannot bump invalid semver version: "${currentVersion}"`);
  }
  const { major, minor, patch } = parsed;
  switch (bumpType.toLowerCase()) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
    default:
      return null;
  }
}

function readAllVersions(): LocationResult[] {
  const results: LocationResult[] = [];
  for (const loc of LOCATIONS) {
    if (!fs.existsSync(loc.file)) {
      results.push({ ...loc, version: null, error: "File not found" });
      continue;
    }
    try {
      const ver = loc.read(loc.file);
      results.push({ ...loc, version: ver, error: ver ? null : "Could not extract version" });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      results.push({ ...loc, version: null, error: msg });
    }
  }
  return results;
}

function reportError(msg: string): void {
  const prefix = process.env.GITHUB_ACTIONS ? "::error::" : "Error: ";
  console.error(`${prefix}${msg}`);
}

function checkVersions(expectedTagOrVersion?: string | null): boolean {
  const versions = readAllVersions();
  const errors: string[] = [];

  for (const v of versions) {
    if (v.error) {
      errors.push(`${v.id}: ${v.error}`);
    }
  }

  const distinctVersions = Object.keys(
    Object.fromEntries(versions.filter((v) => v.version).map((v) => [v.version as string, true]))
  );

  if (distinctVersions.length === 0) {
    reportError("No version numbers found across project files.");
    return false;
  }

  if (distinctVersions.length > 1) {
    const lines = [
      "Version mismatch across project files:",
      ...versions.map((v) => `  - ${v.id}: ${v.version || `(${v.error})`}`),
    ];
    reportError(lines.join("\n"));
    return false;
  }

  const currentVersion = distinctVersions[0];

  if (expectedTagOrVersion) {
    const rawExpected = String(expectedTagOrVersion).trim();
    const expected = rawExpected.startsWith("v")
      ? rawExpected.slice(1)
      : rawExpected;
    const expectedBase = expected.split("-")[0];

    if (currentVersion !== expected && currentVersion !== expectedBase) {
      reportError(
        `Version mismatch: target is '${rawExpected}' (version '${expected}'), but project files define '${currentVersion}'`
      );
      return false;
    }

    console.log(
      `✓ All ${LOCATIONS.length} project locations match version ${currentVersion} (tag '${rawExpected}').`
    );
  } else {
    console.log(
      `✓ All ${LOCATIONS.length} project locations match version ${currentVersion}.`
    );
  }

  return true;
}

function bumpVersions(targetVersion: string, options: BumpOptions = {}): boolean {
  const { dryRun = false } = options;
  const versions = readAllVersions();

  const currentVersion =
    versions.find((v) => v.version && !v.error)?.version || "0.0.0";

  let newVersion = calculateBump(currentVersion, targetVersion);
  if (!newVersion) {
    const parsed = parseSemver(targetVersion);
    if (!parsed) {
      reportError(
        `Invalid version or bump type: '${targetVersion}'. Expected major, minor, patch, or semver (e.g. 0.2.0, v0.2.0).`
      );
      return false;
    }
    newVersion = parsed.normalized;
  }

  console.log(`Bumping version: ${currentVersion} -> ${newVersion}${dryRun ? " (dry run)" : ""}`);

  const versionById: Record<string, string> = Object.fromEntries(
    versions.map((v) => [v.id, v.version || currentVersion])
  );

  for (const loc of LOCATIONS) {
    const oldVer = versionById[loc.id] || currentVersion;
    if (dryRun) {
      console.log(`  [dry-run] Update ${loc.id}: ${oldVer} -> ${newVersion}`);
      continue;
    }

    loc.write(loc.file, newVersion);
    console.log(`  ✓ Updated ${loc.id} to ${newVersion}`);
  }

  if (dryRun) {
    return true;
  }

  // Re-verify after writing
  const check = checkVersions(newVersion);
  if (!check) {
    logError("Post-bump verification failed!");
    return false;
  }

  console.log(`\nSuccessfully bumped all ${LOCATIONS.length} locations to ${newVersion}.`);
  return true;
}

function showHelp(): void {
  console.log(`
Usage:
  node scripts/bump-version.ts <version | major | minor | patch> [options]
  node scripts/bump-version.ts --check [tag_or_version]

Arguments:
  <version>               Explicit version (e.g. 0.2.0, v0.2.0, 0.2.0-rc.1)
  major | minor | patch   Bump the corresponding semver component

Options:
  --check, -c             Verify that all locations match each other and optional expected tag/version
  --dry-run, -n           Simulate the bump without modifying files
  --help, -h              Show this help message

Locations updated:
  - App/package.json
  - App/src-tauri/tauri.conf.json
  - App/src-tauri/Cargo.toml
  - Native/Cargo.toml
  - App/src-tauri/Cargo.lock (litematica-preview)
  - App/src-tauri/Cargo.lock (litematica-preview-native)
  - Native/Cargo.lock (litematica-preview-native)
`);
}

function main(): void {
  const args = process.argv.slice(2);
  if (args.length === 0 || args.includes("--help") || args.includes("-h")) {
    showHelp();
    const versions = readAllVersions();
    console.log("Current versions:");
    for (const v of versions) {
      console.log(`  ${v.id}: ${v.version || `(${v.error})`}`);
    }
    process.exit(args.length === 0 ? 1 : 0);
  }

  const checkIndex = args.findIndex((a) => a === "--check" || a === "-c");
  if (checkIndex !== -1) {
    const remaining = args.filter((_, i) => i !== checkIndex);
    const expected = remaining[0] || null;
    const ok = checkVersions(expected);
    process.exit(ok ? 0 : 1);
  }

  const dryRun = args.includes("--dry-run") || args.includes("-n");
  const targetArg = args.find((a) => !a.startsWith("-"));

  if (!targetArg) {
    reportError("No version or bump type specified.");
    showHelp();
    process.exit(1);
  }

  const ok = bumpVersions(targetArg, { dryRun });
  process.exit(ok ? 0 : 1);
}

main();
