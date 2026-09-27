// Records matching host and decoder processes until interrupted. It only observes
// processes; it never starts the application. Run it through the package script:
// `pnpm run record-memory -- --process LitematicaPreview`.

import { spawn, spawnSync, type ChildProcess } from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import process from "node:process"
import { fileURLToPath } from "node:url"

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const MIB = 1024 * 1024
const WINDOW_SECONDS = 5
// statm counts pages; every mainstream Linux build uses 4 KiB pages.
const PAGE_SIZE = 4096

interface Options {
  intervalMs: number
  processName: string
  outputPath: string
}

interface Sample {
  pids: number[]
  privateBytes: number
  workingBytes: number
}

interface Sampler {
  sample(): Promise<Sample>
  close(): void
}

interface MetricView {
  label: string
  current: number
  minimum: number
  maximum: number
  average: number
  windowAverage: number
}

class Metric {
  private readonly window: { time: number; bytes: number }[] = []
  private minimumBytes = Number.POSITIVE_INFINITY
  private maximumBytes = 0
  private sumBytes = 0
  private windowBytes = 0
  private sampleCount = 0

  constructor(private readonly label: string) {}

  add(elapsed: number, bytes: number): void {
    this.sampleCount += 1
    this.sumBytes += bytes
    this.minimumBytes = Math.min(this.minimumBytes, bytes)
    this.maximumBytes = Math.max(this.maximumBytes, bytes)
    this.window.push({ time: elapsed, bytes })
    this.windowBytes += bytes
    while (this.window.length > 0 && elapsed - this.window[0].time > WINDOW_SECONDS) {
      this.windowBytes -= this.window.shift()?.bytes ?? 0
    }
  }

  view(current: number): MetricView {
    return {
      label: this.label,
      current,
      minimum: this.sampleCount === 0 ? 0 : this.minimumBytes,
      maximum: this.maximumBytes,
      average: this.sampleCount === 0 ? 0 : this.sumBytes / this.sampleCount,
      windowAverage: this.window.length === 0 ? 0 : this.windowBytes / this.window.length,
    }
  }

  summary(): string {
    const average = this.sampleCount === 0 ? 0 : this.sumBytes / this.sampleCount
    return `${this.label} MiB: min=${mib(this.minimumBytes)} max=${mib(this.maximumBytes)} avg=${mib(average)}`
  }
}

class PowerShellSampler implements Sampler {
  private readonly child: ChildProcess
  private readonly buffered: string[] = []
  private readonly readers: ((line: string | null) => void)[] = []
  private failure: Error | undefined

  constructor(processName: string) {
    const script = [
      "$ErrorActionPreference = 'Stop'",
      "$name = $args[0]",
      "while ($null -ne ($line = [Console]::In.ReadLine())) {",
      "  $private = 0.0",
      "  $working = 0.0",
      "  $ids = New-Object 'System.Collections.Generic.List[int]'",
      "  foreach ($target in [System.Diagnostics.Process]::GetProcessesByName($name)) {",
      "    try {",
      "      $target.Refresh()",
      "      if (-not $target.HasExited) {",
      "        $private += $target.PrivateMemorySize64",
      "        $working += $target.WorkingSet64",
      "        $ids.Add($target.Id)",
      "      }",
      "    } catch [System.InvalidOperationException] {",
      "      # A process can exit between enumeration and counter reads.",
      "    } catch {",
      "      [Console]::Error.WriteLine($_.Exception.Message)",
      "    } finally {",
      "      $target.Dispose()",
      "    }",
      "  }",
      "  Write-Output ('{0}|{1:F0}|{2:F0}' -f ([string]::Join(',', $ids)), $private, $working)",
      "}",
    ].join("\n")
    this.child = spawn(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script, processName],
      {
        stdio: ["pipe", "pipe", "pipe"],
        windowsHide: true,
      },
    )
    this.child.stderr?.on("data", (chunk: Buffer) => process.stderr.write(chunk))
    this.child.stdout?.on("data", (chunk: Buffer) => {
      this.buffered.push(...chunk.toString("utf8").split(/\r?\n/))
      this.deliver()
    })
    this.child.on("error", (cause: Error) => this.fail(cause))
    this.child.on("close", (code) =>
      this.fail(new Error(`The PowerShell sampler exited with code ${code ?? "unknown"}.`)),
    )
  }

  private fail(cause: Error): void {
    this.failure = cause
    for (const reader of this.readers.splice(0)) {
      reader(null)
    }
  }

  private deliver(): void {
    while (this.buffered.length > 0 && this.readers.length > 0) {
      this.readers.shift()?.(this.buffered.shift() ?? "")
    }
  }

  private readLine(): Promise<string | null> {
    if (this.failure) {
      return Promise.resolve(null)
    }
    const { promise, resolve } = Promise.withResolvers<string | null>()
    this.readers.push(resolve)
    this.deliver()
    return promise
  }

  async sample(): Promise<Sample> {
    if (this.failure) {
      throw this.failure
    }
    this.child.stdin?.write("\n")
    const line = await this.readLine()
    if (line === null) {
      throw this.failure ?? new Error("The PowerShell sampler stopped responding.")
    }
    const [pidText, privateBytes, workingBytes] = line.split("|")
    return {
      pids: pidText.length > 0 ? pidText.split(",").map(Number) : [],
      privateBytes: Number(privateBytes),
      workingBytes: Number(workingBytes),
    }
  }

  close(): void {
    this.child.stdin?.end()
    this.child.kill()
  }
}

class PosixSampler implements Sampler {
  constructor(private readonly processName: string) {}

  async sample(): Promise<Sample> {
    const pids = run("pgrep", ["-x", this.processName])
      .split("\n")
      .map((line) => Number(line.trim()))
      .filter((pid) => Number.isInteger(pid) && pid > 0)
    const sample: Sample = { pids: [], privateBytes: 0, workingBytes: 0 }
    for (const pid of pids) {
      try {
        const { privateBytes, workingBytes } = posixMemory(pid)
        sample.pids.push(pid)
        sample.privateBytes += privateBytes
        sample.workingBytes += workingBytes
      } catch {
        // A process can exit between enumeration and counter reads.
      }
    }
    return sample
  }

  close(): void {}
}

function run(tool: string, args: string[]): string {
  const result = spawnSync(tool, args, { encoding: "utf8" })
  if (result.error) {
    throw new Error(`${tool} is not available on this host: ${result.error.message}`)
  }
  return result.stdout ?? ""
}

function posixMemory(pid: number): { privateBytes: number; workingBytes: number } {
  if (process.platform === "linux") {
    // statm reports pages: size resident shared text lib data dt.
    const statm = fs.readFileSync(`/proc/${pid}/statm`, "utf8").split(" ")
    const resident = Number(statm[1]) * PAGE_SIZE
    return { privateBytes: resident - Number(statm[2]) * PAGE_SIZE, workingBytes: resident }
  }
  const rss = run("ps", ["-o", "rss=", "-p", String(pid)]).trim()
  if (rss.length === 0) {
    throw new Error(`Process ${pid} is no longer readable.`)
  }
  return { privateBytes: Number(rss) * 1024, workingBytes: Number(rss) * 1024 }
}

function mib(bytes: number): string {
  return (bytes / MIB).toFixed(2)
}

function createSampler(processName: string): Sampler {
  if (process.platform === "win32") {
    return new PowerShellSampler(processName)
  }
  if (process.platform === "linux" || process.platform === "darwin") {
    return new PosixSampler(processName)
  }
  throw new Error(`Memory recording is not supported on ${process.platform}.`)
}

function parseArguments(argv: string[]): Options | null {
  const options: Options = {
    intervalMs: 200,
    processName: "LitematicaPreview",
    outputPath: path.join(ROOT, "artifacts", "memory-recording.csv"),
  }
  const flags: Record<string, keyof Options> = {
    "--interval": "intervalMs",
    "--process": "processName",
    "--output": "outputPath",
  }
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index]
    if (flag === "--help") {
      console.log(`Usage: pnpm run record-memory -- [options]

Options:
  --interval <ms>    Sampling interval in milliseconds (100-5000, default 200).
  --process <name>   Process name without extension (default LitematicaPreview).
  --output <path>    CSV destination (default artifacts/memory-recording.csv).

An existing recording is never overwritten.`)
      return null
    }
    const key = flags[flag]
    const value = argv[index + 1]
    if (!key || value === undefined) {
      throw new Error(`Unknown or incomplete argument: ${flag}`)
    }
    if (key === "intervalMs") {
      const interval = Number(value)
      if (!Number.isInteger(interval) || interval < 100 || interval > 5000) {
        throw new Error(`--interval must be an integer between 100 and 5000, received: ${value}`)
      }
      options.intervalMs = interval
    } else if (key === "processName") {
      options.processName = path.basename(value, path.extname(value))
    } else {
      options.outputPath = path.resolve(value)
    }
    index++
  }
  return options
}

function renderRow(pids: string, stamp: string, view: MetricView): string {
  return [
    stamp.padEnd(12),
    pids.padEnd(20),
    view.label.padEnd(10),
    mib(view.current).padStart(12),
    mib(view.minimum).padStart(11),
    mib(view.maximum).padStart(11),
    mib(view.average).padStart(11),
    mib(view.windowAverage).padStart(11),
  ].join(" ")
}

async function main(): Promise<void> {
  const options = parseArguments(process.argv.slice(2))
  if (!options) {
    return
  }

  let fileDescriptor: number
  try {
    // wx refuses to open an existing file, so a recording is never overwritten.
    fileDescriptor = fs.openSync(options.outputPath, "wx")
  } catch (cause) {
    const failure = cause as NodeJS.ErrnoException
    throw new Error(
      failure.code === "EEXIST"
        ? `Refusing to overwrite an existing recording: ${options.outputPath}`
        : `Cannot open the recording ${options.outputPath}: ${failure.message}`,
    )
  }
  fs.writeSync(
    fileDescriptor,
    "timestamp,elapsed_seconds,process_count,pids,private_bytes,working_set_bytes\n",
  )

  const metrics = [new Metric("Private"), new Metric("WorkingSet")]
  const sampler = createSampler(options.processName)
  const startedAt = Date.now()
  const { promise: stopRequested, resolve: requestStop } = Promise.withResolvers<void>()
  let interrupted = false
  process.on("SIGINT", () => {
    interrupted = true
    requestStop()
  })

  console.log(
    `Recording ${options.processName} processes every ${options.intervalMs} ms. Ctrl+C to stop.`,
  )
  console.log(`CSV: ${options.outputPath}`)
  console.log(
    "All matching processes are summed (host + decoder). WebView2/GPU processes are excluded.",
  )
  if (process.platform !== "win32") {
    console.log("This host reports the resident set; only Windows exposes a private-bytes counter.")
  }
  console.log(
    `${WINDOW_SECONDS}s avg is the arithmetic mean of samples from the last ${WINDOW_SECONDS} seconds. Units: MiB.`,
  )
  console.log()
  console.log(
    "Time         PIDs                 Metric       Current         Min         Max         Avg      5s Avg",
  )

  let waiting = false
  let samples = 0
  try {
    while (!interrupted) {
      const iteration = Date.now()
      const sample = await sampler.sample()
      const elapsed = (iteration - startedAt) / 1000
      if (sample.pids.length === 0) {
        if (!waiting) {
          const clock = new Date().toISOString().slice(11, 19)
          console.log(`[${clock}] Waiting for readable ${options.processName} processes...`)
          waiting = true
        }
      } else {
        waiting = false
        samples += 1
        metrics[0].add(elapsed, sample.privateBytes)
        metrics[1].add(elapsed, sample.workingBytes)
        const pids = [...sample.pids].sort((left, right) => left - right).join(";")
        const timestamp = new Date().toISOString()
        fs.writeSync(
          fileDescriptor,
          `${timestamp},${elapsed.toFixed(3)},${sample.pids.length},${pids},` +
            `${Math.round(sample.privateBytes)},${Math.round(sample.workingBytes)}\n`,
        )
        const stamp = timestamp.slice(11, 23)
        for (const [index, bytes] of [sample.privateBytes, sample.workingBytes].entries()) {
          console.log(renderRow(pids, stamp, metrics[index].view(bytes)))
        }
      }
      const remaining = options.intervalMs - (Date.now() - iteration)
      if (remaining > 0) {
        const pause = Promise.withResolvers<void>()
        setTimeout(pause.resolve, remaining)
        await Promise.race([pause.promise, stopRequested])
      }
    }
  } finally {
    sampler.close()
    fs.closeSync(fileDescriptor)
  }

  if (samples > 0) {
    console.log(`Stopped. ${samples} samples recorded.`)
    for (const metric of metrics) {
      console.log(metric.summary())
    }
  }
}

try {
  await main()
} catch (cause) {
  console.error(`Error: ${(cause as Error).message}`)
  process.exitCode = 1
}
