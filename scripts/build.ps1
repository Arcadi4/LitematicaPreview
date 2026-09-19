param([switch]$Installer)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build the native Windows DLL on Windows with the MSVC toolchain.' }
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $originalFlags = $env:RUSTFLAGS
    try {
        # Ship without a separate Visual C++ runtime installer.
        if ($env:RUSTFLAGS -notlike '*target-feature=+crt-static*') {
            $env:RUSTFLAGS = if ($originalFlags) { "$originalFlags -C target-feature=+crt-static" } else { "-C target-feature=+crt-static" }
        }
        & cargo build --manifest-path Native/Cargo.toml --release --locked --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'Native build failed.' }
    } finally { $env:RUSTFLAGS = $originalFlags }
    & dotnet publish App/LitematicaPreview.csproj -c Release -r win-x64 --self-contained true -p:RestoreLockedMode=true -o artifacts/win-x64
    if ($LASTEXITCODE -ne 0) { throw 'C# publish failed.' }
    if (!(Test-Path artifacts/win-x64/litematica_preview_native.dll)) { throw 'Native DLL is missing from the package.' }
    if (!(Test-Path artifacts/win-x64/glfw3.dll)) { throw 'The OpenTK Windows runtime is missing from the package.' }
    if ($Installer) {
        $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        $iscc = if ($compiler) { $compiler.Source } else { "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }
        & $iscc scripts/installer.iss
        if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
    }
} finally { Pop-Location }
