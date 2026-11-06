export type FlyDirection = { forward: number; right: number; up: number }

const flyKeys: Record<string, "forward" | "backward" | "left" | "right" | "up" | "down"> = {
  KeyW: "forward",
  KeyS: "backward",
  KeyA: "left",
  KeyD: "right",
  Space: "up",
  ControlLeft: "down",
  ControlRight: "down",
}

export const maxFlyStep = 0.1

export function isFlyKey(code: string): boolean {
  return Object.hasOwn(flyKeys, code)
}

export function flyDirection(pressed: ReadonlySet<string>): FlyDirection {
  let forward = 0
  let backward = 0
  let left = 0
  let right = 0
  let up = 0
  let down = 0
  for (const code of pressed) {
    switch (flyKeys[code]) {
      case "forward":
        forward = 1
        break
      case "backward":
        backward = 1
        break
      case "left":
        left = 1
        break
      case "right":
        right = 1
        break
      case "up":
        up = 1
        break
      case "down":
        down = 1
        break
      default:
        break
    }
  }
  return { forward: forward - backward, right: right - left, up: up - down }
}

export function flySeconds(now: number, previous: number): number {
  return Math.min(Math.max(now - previous, 0) / 1000, maxFlyStep)
}

/**
 * Emits one time-scaled move per display frame while any fly key is held.
 * The animation frame exists only for the duration of the hold, so an idle
 * spectator camera schedules no work.
 */
export class FlyMotion {
  private pressed = new Set<string>()
  private frame: number | null = null
  private last = 0

  constructor(
    private readonly move: (direction: FlyDirection, seconds: number) => void,
    private readonly now: () => number = () => performance.now(),
  ) {}

  /** Presses a fly key; returns false for unrelated codes. */
  press(code: string): boolean {
    if (!isFlyKey(code)) return false
    this.pressed.add(code)
    if (this.frame === null) {
      this.last = this.now()
      this.frame = requestAnimationFrame(this.step)
    }
    return true
  }

  release(code: string): void {
    if (this.pressed.delete(code) && this.pressed.size === 0) this.stop()
  }

  stop(): void {
    this.pressed.clear()
    if (this.frame !== null) cancelAnimationFrame(this.frame)
    this.frame = null
  }

  private readonly step = (now: number): void => {
    this.frame = null
    const direction = flyDirection(this.pressed)
    const seconds = flySeconds(now, this.last)
    this.last = now
    if (seconds > 0 && (direction.forward || direction.right || direction.up))
      this.move(direction, seconds)
    if (this.pressed.size > 0) this.frame = requestAnimationFrame(this.step)
  }
}

export function requestPointerLock(element: HTMLElement | null): void {
  if (!element) return
  try {
    const pending: unknown = element.requestPointerLock()
    if (pending instanceof Promise) void pending.catch(() => {})
  } catch {
    // Drag-to-look remains available when the browser refuses pointer lock.
  }
}

export function releasePointerLock(): void {
  if (document.pointerLockElement) document.exitPointerLock()
}
