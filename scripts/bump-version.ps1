param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Arguments
)
$ErrorActionPreference = 'Stop'
$script = Join-Path $PSScriptRoot 'bump-version.ts'
& node $script @Arguments
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
