param(
    # Retained for existing callers; setup and portable packages are always built.
    [switch]$Installer,
    [switch]$NonInteractive
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path $PSScriptRoot -Parent
$target = 'x86_64-pc-windows-msvc'
$tauriRoot = Join-Path $root 'App/src-tauri'
$targetRoot = Join-Path $tauriRoot 'target'
$releaseRoot = Join-Path $targetRoot "$target/release"
$artifactsRoot = Join-Path $root 'artifacts'
$portableRoot = Join-Path $artifactsRoot 'win-x64'
$installerRoot = Join-Path $releaseRoot 'bundle/nsis'
$logsRoot = Join-Path $artifactsRoot 'logs'
$logPath = Join-Path $logsRoot ("build-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$originalFlags = $env:RUSTFLAGS
$originalTargetDir = $env:CARGO_TARGET_DIR
$originalCI = $env:CI
$elapsed = [System.Diagnostics.Stopwatch]::StartNew()
$stage = 'Preparing build log'
$transcribing = $false
$locationPushed = $false
$interactive = $false

try {
    New-Item -ItemType Directory -Path $logsRoot -Force | Out-Null
    Start-Transcript -LiteralPath $logPath | Out-Null
    $transcribing = $true
    Write-Host 'Litematica Preview - Windows x64 release build' -ForegroundColor Cyan
    Write-Host "Build log: $logPath"

    $stage = 'Checking prerequisites'
    Write-Host "`n[1/5] $stage" -ForegroundColor Cyan
    if ($env:OS -ne 'Windows_NT') {
        throw 'Run this script on Windows with Visual Studio C++ Build Tools, the Windows SDK, and the Rust MSVC toolchain.'
    }
    $interactive = -not $NonInteractive -and -not $env:CI -and -not $env:GITHUB_ACTIONS -and -not $env:TF_BUILD -and
        [Environment]::UserInteractive -and $Host.Name -eq 'ConsoleHost' -and
        -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected -and
        -not ([Environment]::GetCommandLineArgs() -contains '-NonInteractive')

    $pnpm = Get-Command pnpm.cmd -ErrorAction SilentlyContinue
    if (-not $pnpm) { $pnpm = Get-Command pnpm -ErrorAction SilentlyContinue }
    if (-not $pnpm) { throw 'pnpm is not on PATH. Install Node.js 24 and run: npm install --global pnpm@12.5.1. Then open a new terminal.' }
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { throw 'cargo is not on PATH. Install Rust from https://rustup.rs, reopen the terminal, then run: rustup target add x86_64-pc-windows-msvc.' }
    $node = Get-Command node -ErrorAction SilentlyContinue
    if (-not $node) { throw 'Node.js is not on PATH. Install Node.js 24 LTS from https://nodejs.org/ and open a new terminal.' }

    Push-Location $root
    $locationPushed = $true
    # Prevent dependency tools from prompting; only the final folder prompt is interactive.
    $env:CI = 'true'
    & $node.Source --version | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Node.js could not start. Reinstall Node.js 24 LTS and reopen the terminal.' }
    & $pnpm.Source --version | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'pnpm could not start. Run: npm install --global pnpm@12.5.1.' }
    & $cargo.Source --version | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Cargo could not start. Repair the Rust MSVC installation using rustup.' }
    Write-Host 'Requires Visual Studio C++ Build Tools, Windows SDK, and: rustup target add x86_64-pc-windows-msvc'

    $config = Get-Content -LiteralPath (Join-Path $tauriRoot 'tauri.conf.json') -Raw | ConvertFrom-Json
    $version = [string]$config.version
    if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
        throw 'App/src-tauri/tauri.conf.json must contain an explicit semantic version for package filenames.'
    }
    $packageName = "LitematicaPreview-$version-win-x64"
    $setupPath = Join-Path $artifactsRoot "$packageName-setup.exe"
    $portableZip = Join-Path $artifactsRoot "$packageName-portable.zip"
    foreach ($resource in $config.bundle.resources.PSObject.Properties) {
        $source = Join-Path $tauriRoot $resource.Name
        if (-not (Test-Path -LiteralPath $source)) { throw "Required bundle resource is missing: $source. Restore the tracked input before building." }
    }
    $existingExecutable = Join-Path $portableRoot 'LitematicaPreview.exe'
    if (Test-Path -LiteralPath $existingExecutable) {
        try {
            $probe = [IO.File]::Open($existingExecutable, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            $probe.Dispose()
        } catch {
            throw "Close the portable Litematica Preview app before building: $existingExecutable"
        }
    }

    # Keep output predictable even when the caller has a shared Cargo target dir.
    $env:CARGO_TARGET_DIR = $targetRoot
    # Ship without a separate Visual C++ runtime installer.
    if ($env:RUSTFLAGS -notlike '*target-feature=+crt-static*') {
        $env:RUSTFLAGS = if ($originalFlags) { "$originalFlags -C target-feature=+crt-static" } else { '-C target-feature=+crt-static' }
    }

    $stage = 'Installing frontend dependencies'
    Write-Host "`n[2/5] $stage" -ForegroundColor Cyan
    & $pnpm.Source --prefix App install --frozen-lockfile | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed. Check the log for network or lockfile errors.' }

    $stage = 'Building the application and NSIS setup'
    Write-Host "`n[3/5] $stage" -ForegroundColor Cyan
    if (Test-Path -LiteralPath $installerRoot) { Remove-Item -LiteralPath $installerRoot -Recurse -Force }
    foreach ($oldPackage in @($setupPath, $portableZip)) {
        if (Test-Path -LiteralPath $oldPackage) { Remove-Item -LiteralPath $oldPackage -Force }
    }
    # Tauri builds the frontend. Arguments after the separator go to Cargo.
    & $pnpm.Source --prefix App exec tauri build --target $target --ci --bundles nsis -- --locked | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Tauri release build failed. Check the compiler/bundler output in the log; confirm the MSVC C++ tools, Windows SDK, and Rust target are installed.' }

    $stage = 'Staging the portable application and setup'
    Write-Host "`n[4/5] $stage" -ForegroundColor Cyan
    $executable = Join-Path $releaseRoot 'LitematicaPreview.exe'
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "The release executable is missing: $executable" }
    $setups = @(Get-ChildItem -LiteralPath $installerRoot -Filter '*-setup.exe' -File)
    if ($setups.Count -ne 1) { throw "Expected one fresh x64 NSIS setup in $installerRoot; found $($setups.Count)." }
    if (Test-Path -LiteralPath $portableRoot) { Remove-Item -LiteralPath $portableRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $portableRoot -Force | Out-Null
    Copy-Item -LiteralPath $executable -Destination $portableRoot

    # Use the same resource map as NSIS. Frontend assets are embedded in the exe.
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
    Copy-Item -LiteralPath $setups[0].FullName -Destination $setupPath -Force

    $stage = 'Creating the portable ZIP'
    Write-Host "`n[5/5] $stage" -ForegroundColor Cyan
    foreach ($file in @('LitematicaPreview.exe', 'Assets/pack.zip', 'Licenses/LICENSE', 'Licenses/NOTICE')) {
        $required = Join-Path $portableRoot $file
        if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required portable file is missing: $required" }
        if ((Get-Item -LiteralPath $required).Length -eq 0) { throw "Required portable file is empty: $required" }
    }
    foreach ($directory in @('Assets', 'Demos', 'Licenses', 'Licenses/ThirdParty')) {
        $required = Join-Path $portableRoot $directory
        if (-not (Test-Path -LiteralPath $required -PathType Container)) { throw "Required portable directory is missing: $required" }
        if (-not (Get-ChildItem -LiteralPath $required -File -Recurse -Force | Select-Object -First 1)) { throw "Required portable directory is empty: $required" }
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::CreateFromDirectory($portableRoot, $portableZip, [System.IO.Compression.CompressionLevel]::Optimal, $false)

    foreach ($package in @($setupPath, $portableZip)) {
        $item = Get-Item -LiteralPath $package
        if ($item.Length -eq 0) { throw "Generated package is empty: $package" }
    }
    $elapsed.Stop()
    Write-Host "`nBuild completed in $($elapsed.Elapsed.ToString('hh\:mm\:ss'))." -ForegroundColor Green
    foreach ($package in @($setupPath, $portableZip)) {
        $item = Get-Item -LiteralPath $package
        Write-Host ("{0}`n  {1:N2} MiB ({2:N0} bytes)" -f $item.FullName, ($item.Length / 1MB), $item.Length)
    }
    Write-Host "Portable staging: $portableRoot"
    Write-Host "Build log: $logPath"
    Write-Host 'Install: run the setup executable. File associations are optional in setup.'
    Write-Host 'Portable: extract the ZIP, then run LitematicaPreview.exe; keep Assets, Demos, and Licenses beside it.'
    Write-Host 'Portable copies require Microsoft Edge WebView2 Evergreen Runtime. Setup installs it if needed.'
    if ($interactive) {
        try {
            if ((Read-Host 'Open the artifacts folder? [y/N]') -match '^(?i:y|yes)$') {
                Start-Process explorer.exe -ArgumentList ('"{0}"' -f $artifactsRoot) | Out-Null
            }
        } catch {
            Write-Warning "The build succeeded, but the artifacts folder could not be opened: $($_.Exception.Message)"
        }
    }
} catch {
    Write-Host "`nBuild failed during: $stage" -ForegroundColor Red
    Write-Host "Elapsed: $($elapsed.Elapsed.ToString('hh\:mm\:ss'))"
    Write-Host "Reason: $($_.Exception.Message)"
    Write-Host "Build log: $logPath"
    throw
} finally {
    $elapsed.Stop()
    $env:RUSTFLAGS = $originalFlags
    $env:CARGO_TARGET_DIR = $originalTargetDir
    $env:CI = $originalCI
    if ($locationPushed) { Pop-Location }
    if ($transcribing) { Stop-Transcript | Out-Null }
}
