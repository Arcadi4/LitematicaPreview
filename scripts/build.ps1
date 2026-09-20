param([switch]$Installer)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build the Windows app on Windows with the Rust MSVC toolchain.' }
$root = Split-Path $PSScriptRoot -Parent
$target = 'x86_64-pc-windows-msvc'
$tauriRoot = Join-Path $root 'App/src-tauri'
$targetRoot = Join-Path $tauriRoot 'target'
$releaseRoot = Join-Path $targetRoot "$target/release"
$portableRoot = Join-Path $root 'artifacts/win-x64'
$installerRoot = Join-Path $releaseRoot 'bundle/nsis'
$originalFlags = $env:RUSTFLAGS
$originalTargetDir = $env:CARGO_TARGET_DIR
Push-Location $root
try {
    # Keep output predictable even when the caller has a shared Cargo target dir.
    $env:CARGO_TARGET_DIR = $targetRoot
    # Ship without a separate Visual C++ runtime installer.
    if ($env:RUSTFLAGS -notlike '*target-feature=+crt-static*') {
        $env:RUSTFLAGS = if ($originalFlags) { "$originalFlags -C target-feature=+crt-static" } else { '-C target-feature=+crt-static' }
    }

    & npm.cmd --prefix App ci
    if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed.' }

    $tauriArgs = @('build', '--target', $target, '--ci')
    if ($Installer) {
        if (Test-Path -LiteralPath $installerRoot) { Remove-Item -LiteralPath $installerRoot -Recurse -Force }
        $tauriArgs += @('--bundles', 'nsis')
    } else {
        $tauriArgs += '--no-bundle'
    }
    # Tauri's beforeBuildCommand builds the frontend. Arguments after the second
    # separator go to Cargo, so --locked applies to the Rust dependency graph.
    $tauriArgs += @('--', '--locked')
    & npm.cmd --prefix App run tauri -- @tauriArgs
    if ($LASTEXITCODE -ne 0) { throw 'Tauri release build failed.' }

    $executable = Join-Path $releaseRoot 'LitematicaPreview.exe'
    if (!(Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'The release executable is missing.' }
    if (Test-Path -LiteralPath $portableRoot) { Remove-Item -LiteralPath $portableRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $portableRoot -Force | Out-Null
    Copy-Item -LiteralPath $executable -Destination $portableRoot

    # Use the same resource map as NSIS; portable and installed copies must resolve
    # identical paths relative to their executable. Frontend assets are embedded.
    $config = Get-Content -LiteralPath (Join-Path $tauriRoot 'tauri.conf.json') -Raw | ConvertFrom-Json
    foreach ($resource in $config.bundle.resources.PSObject.Properties) {
        $source = Join-Path $tauriRoot $resource.Name
        $destination = Join-Path $portableRoot $resource.Value
        if (Test-Path -LiteralPath $source -PathType Container) {
            New-Item -ItemType Directory -Path $destination -Force | Out-Null
            Get-ChildItem -LiteralPath $source -Force | Copy-Item -Destination $destination -Recurse -Force
        } else {
            New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
            Copy-Item -LiteralPath $source -Destination $destination -Force
        }
    }

    if ($Installer) {
        $setups = @(Get-ChildItem -LiteralPath $installerRoot -Filter '*-setup.exe' -File)
        if ($setups.Count -eq 0) { throw 'The NSIS setup executable is missing.' }
        foreach ($setup in $setups) {
            Copy-Item -LiteralPath $setup.FullName -Destination (Join-Path $root 'artifacts') -Force
        }
    }
    Write-Host "Portable app: $portableRoot"
    if ($Installer) { Write-Host "NSIS installer: $(Join-Path $root 'artifacts/*-setup.exe')" }
} finally {
    $env:RUSTFLAGS = $originalFlags
    $env:CARGO_TARGET_DIR = $originalTargetDir
    Pop-Location
}
