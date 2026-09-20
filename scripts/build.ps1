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

    $pnpm = if (Get-Command pnpm.cmd -ErrorAction SilentlyContinue) { 'pnpm.cmd' } else { 'pnpm' }
    & $pnpm --prefix App install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed.' }

    if (Test-Path -LiteralPath $installerRoot) { Remove-Item -LiteralPath $installerRoot -Recurse -Force }
    $tauriArgs = @('build', '--target', $target, '--ci', '--bundles', 'nsis')
    # Tauri's beforeBuildCommand builds the frontend. Arguments after the second
    # separator go to Cargo, so --locked applies to the Rust dependency graph.
    $tauriArgs += @('--', '--locked')
    & $pnpm --prefix App exec tauri @tauriArgs
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

    $packageName = "LitematicaPreview-$($config.version)-win-x64"
    $setupPath = Join-Path $root "artifacts/$packageName-setup.exe"
    $portableZip = Join-Path $root "artifacts/$packageName-portable.zip"
    $setups = @(Get-ChildItem -LiteralPath $installerRoot -Filter '*-setup.exe' -File)
    if ($setups.Count -ne 1) { throw 'Expected one fresh NSIS setup executable.' }
    Copy-Item -LiteralPath $setups[0].FullName -Destination $setupPath -Force
    foreach ($file in @('LitematicaPreview.exe', 'Assets/pack.zip', 'Licenses/LICENSE', 'Licenses/NOTICE')) {
        $required = Join-Path $portableRoot $file
        if (!(Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing portable file: $required" }
    }
    if (Test-Path -LiteralPath $portableZip) { Remove-Item -LiteralPath $portableZip -Force }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($portableRoot, $portableZip, [System.IO.Compression.CompressionLevel]::Optimal, $false)
    Write-Host "Setup: $setupPath"
    Write-Host "Portable ZIP: $portableZip"
} finally {
    $env:RUSTFLAGS = $originalFlags
    $env:CARGO_TARGET_DIR = $originalTargetDir
    Pop-Location
}
