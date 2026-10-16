import type { ViewportCommand } from "./app-model"

/** One frame is in flight; input received meanwhile joins the next frame. */
export class ViewportInputQueue {
  private pending: ViewportCommand[] = []
  private frame: number | null = null
  private inFlight = false
  private generation = 0

  constructor(
    private readonly send: (commands: ViewportCommand[]) => Promise<void>,
    private readonly onError: (error: unknown) => void,
  ) {}

  push(command: ViewportCommand): void {
    this.pending.push(command)
    this.schedule()
  }

  clear(): void {
    this.generation++
    this.pending = []
    if (this.frame !== null) cancelAnimationFrame(this.frame)
    this.frame = null
  }

  private schedule(): void {
    if (this.frame !== null || this.inFlight || this.pending.length === 0) return
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
