import type { ViewportCommand } from "./app-model"

/** Starts paused; coalesces durable state and preserves motion order with one frame in flight. */
export class ViewportInputQueue {
  private pending: ViewportCommand[] = []
  private grid: Extract<ViewportCommand, { kind: "grid" }> | null = null
  private stage: Extract<ViewportCommand, { kind: "stage" }> | null = null
  private ready = false
  private frame: number | null = null
  private inFlight = false
  private generation = 0

  constructor(
    private readonly send: (commands: ViewportCommand[]) => Promise<void>,
    private readonly onError: (error: unknown) => void,
  ) {}

  push(command: ViewportCommand): void {
    if (command.kind === "grid") this.grid = command
    else if (command.kind === "stage") this.stage = command
    if (!this.ready) return
    if (command.kind === "grid" || command.kind === "stage") {
      const previous = this.pending.findIndex((pending) => pending.kind === command.kind)
      if (previous !== -1) this.pending.splice(previous, 1)
    }
    this.pending.push(command)
    this.schedule()
  }

  /** Hold durable state until native initialization confirms a usable viewport. */
  setReady(ready: boolean): void {
    if (this.ready === ready) return
    this.ready = ready
    if (ready) this.resetScene()
    else this.clearPending()
  }

  /** A scene reset drops movement, not the grid preference or stage layout. */
  resetScene(): void {
    this.clearPending()
    if (this.grid) this.pending.push(this.grid)
    if (this.stage) this.pending.push(this.stage)
    this.schedule()
  }

  /** Full session teardown must not dispatch cached state after unmount. */
  clear(): void {
    this.ready = false
    this.grid = null
    this.stage = null
    this.clearPending()
  }

  private clearPending(): void {
    this.generation++
    this.pending = []
    if (this.frame !== null) cancelAnimationFrame(this.frame)
    this.frame = null
  }

  private schedule(): void {
    if (!this.ready || this.frame !== null || this.inFlight || this.pending.length === 0) return
    this.frame = requestAnimationFrame(() => {
      this.frame = null
      const commands = this.pending
      this.pending = []
      const generation = this.generation
      this.inFlight = true
      void this.send(commands)
        .catch((error: unknown) => {
          if (generation === this.generation) {
            this.pending = []
            this.onError(error)
          }
        })
        .finally(() => {
          this.inFlight = false
          this.schedule()
        })
    })
  }
}
