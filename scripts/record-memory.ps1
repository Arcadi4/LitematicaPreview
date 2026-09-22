# Continuous memory recording for the host and decoder processes.
# Run from any directory. Stop with Ctrl+C. Does not start the application.
[CmdletBinding()]
param(
    [ValidateRange(100, 5000)]
    [int]$IntervalMs = 1000,

    [string]$ProcessName = 'LitematicaPreview',

    [string]$OutputPath = (Join-Path $PSScriptRoot '../artifacts/memory-recording.csv')
)

$ErrorActionPreference = 'Stop'
$ProcessName = [System.IO.Path]::GetFileNameWithoutExtension($ProcessName)
$outputFile = [System.IO.Path]::GetFullPath($OutputPath)
$null = [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($outputFile))
$culture = [System.Globalization.CultureInfo]::InvariantCulture
$window = [System.Collections.Generic.Queue[object]]::new()
$clock = [System.Diagnostics.Stopwatch]::StartNew()
$writer = $null
$count = 0L
$privateSum = 0.0
$workingSum = 0.0
$privateWindowSum = 0.0
$workingWindowSum = 0.0
$privateMin = [double]::PositiveInfinity
$workingMin = [double]::PositiveInfinity
$privateMax = 0.0
$workingMax = 0.0
$waiting = $false

function Format-MiB([double]$Bytes) {
    return ($Bytes / 1MB).ToString('F2', $culture)
}

try {
    # CreateNew prevents overwriting an earlier recording accidentally.
    $stream = [System.IO.FileStream]::new(
        $outputFile, [System.IO.FileMode]::CreateNew,
        [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read
    )
    $writer = [System.IO.StreamWriter]::new($stream, [System.Text.UTF8Encoding]::new($false))
    $writer.AutoFlush = $true
    $writer.WriteLine('timestamp,elapsed_seconds,process_count,pids,private_bytes,working_set_bytes')

    Write-Host "Recording $ProcessName.exe every $IntervalMs ms. Ctrl+C to stop."
    Write-Host "CSV: $outputFile"
    Write-Host 'All matching processes are summed (host + decoder). WebView2/GPU processes are excluded.'
    Write-Host 'Statistics use successful samples; absent processes are not counted as zero.'
    Write-Host '5s avg is the arithmetic mean of samples from the last 5 seconds. Units: MiB.'
    Write-Host ''
    Write-Host 'Time         PIDs                 Metric       Current         Min         Max         Avg      5s Avg'

    while ($true) {
        $private = 0.0
        $working = 0.0
        $ids = [System.Collections.Generic.List[int]]::new()
        $processes = [System.Diagnostics.Process]::GetProcessesByName($ProcessName)
        foreach ($process in $processes) {
            try {
                $process.Refresh()
                $processId = $process.Id
                $privateBytes = $process.PrivateMemorySize64
                $workingBytes = $process.WorkingSet64
                if (-not $process.HasExited) {
                    $private += $privateBytes
                    $working += $workingBytes
                    $ids.Add($processId)
                }
            }
            catch [System.InvalidOperationException] {
                # Process exited while its counters were being read.
            }
            catch [System.ComponentModel.Win32Exception] {
                Write-Warning "Cannot read a matching process: $($_.Exception.Message)"
            }
            finally {
                $process.Dispose()
            }
        }

        $elapsed = $clock.Elapsed.TotalSeconds
        while ($window.Count -gt 0 -and $window.Peek().Time -le ($elapsed - 5.0)) {
            $expired = $window.Dequeue()
            $privateWindowSum -= $expired.Private
            $workingWindowSum -= $expired.Working
        }

        if ($ids.Count -eq 0) {
            if (-not $waiting) {
                Write-Host "[$(Get-Date -Format 'HH:mm:ss')] Waiting for readable $ProcessName.exe processes..."
                $waiting = $true
            }
        }
        else {
            $waiting = $false
            $timestamp = [DateTimeOffset]::Now
            $ids.Sort()
            $pidText = $ids -join ';'
            $count++
            $privateSum += $private
            $workingSum += $working
            $privateMin = [Math]::Min($privateMin, $private)
            $workingMin = [Math]::Min($workingMin, $working)
            $privateMax = [Math]::Max($privateMax, $private)
            $workingMax = [Math]::Max($workingMax, $working)
            $window.Enqueue([pscustomobject]@{ Time = $elapsed; Private = $private; Working = $working })
            $privateWindowSum += $private
            $workingWindowSum += $working

            $writer.WriteLine(('{0},{1},{2},{3},{4},{5}' -f
                $timestamp.ToString('o', $culture), $elapsed.ToString('F3', $culture),
                $ids.Count, $pidText, $private.ToString('F0', $culture), $working.ToString('F0', $culture)))

            foreach ($row in @(
                @('Private', $private, $privateMin, $privateMax, ($privateSum / $count), ($privateWindowSum / $window.Count)),
                @('WorkingSet', $working, $workingMin, $workingMax, ($workingSum / $count), ($workingWindowSum / $window.Count))
            )) {
                Write-Host ('{0,-12} {1,-20} {2,-10} {3,12} {4,11} {5,11} {6,11} {7,11}' -f
                    $timestamp.ToString('HH:mm:ss.fff'), $pidText, $row[0],
                    (Format-MiB $row[1]), (Format-MiB $row[2]), (Format-MiB $row[3]),
                    (Format-MiB $row[4]), (Format-MiB $row[5]))
            }
        }
        Start-Sleep -Milliseconds $IntervalMs
    }
}
finally {
    if ($null -ne $writer) {
        $writer.Dispose()
    }
    $clock.Stop()
    if ($count -gt 0) {
        Write-Host "Stopped. $count samples recorded."
        Write-Host ("Private MiB: min={0} max={1} avg={2}" -f
            (Format-MiB $privateMin), (Format-MiB $privateMax), (Format-MiB ($privateSum / $count)))
        Write-Host ("WorkingSet MiB: min={0} max={1} avg={2}" -f
            (Format-MiB $workingMin), (Format-MiB $workingMax), (Format-MiB ($workingSum / $count)))
    }
}
