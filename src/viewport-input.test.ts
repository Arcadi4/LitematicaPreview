import assert from "node:assert/strict"
import { afterEach, beforeEach, test, vi } from "vite-plus/test"
import type { ViewportCommand } from "./app-model"
import { ViewportInputQueue } from "./viewport-input"

let frames: Map<number, FrameRequestCallback>

beforeEach(() => {
  frames = new Map()
  let id = 0
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++id, callback)
    return id
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id))
})
afterEach(() => vi.unstubAllGlobals())

function frame() {
  const callbacks = [...frames.values()]
  frames.clear()
  for (const callback of callbacks) callback(0)
}
/** Flushes the queue's promise handlers without waiting on a timer. */
function settle() {
  return new Promise<void>((resolve) => queueMicrotask(resolve))
}

/** Takes the awaited completion; missing sends fail the test. */
function nextCompletion(completions: (() => void)[]) {
  const complete = completions.shift()
  assert.ok(complete, "Expected a pending frame completion")
  return complete
}

function viewportState() {
  return {
    grid: true,
    stage: null as Extract<ViewportCommand, { kind: "stage" }> | null,
    motion: [] as ViewportCommand[],
    apply(commands: ViewportCommand[]) {
      for (const command of commands) {
        if (command.kind === "grid") this.grid = command.visible
        else if (command.kind === "stage") this.stage = command
        else this.motion.push(command)
      }
    },
  }
}

test("before readiness only the latest grid and stage survive", async () => {
  const state = viewportState()
  const sent: ViewportCommand[][] = []
  const queue = new ViewportInputQueue(
    async (commands) => {
      sent.push(commands)
      state.apply(commands)
    },
    (error) => assert.fail(String(error)),
  )
  queue.push({ kind: "grid", visible: true })
  queue.push({ kind: "stage", left: 0, top: 0, width: 1, height: 1 })
  queue.push({ kind: "orbit", dx: 20, dy: -10 })
  const stage: ViewportCommand = { kind: "stage", left: 12, top: 80, width: 800, height: 500 }
  queue.push({ kind: "grid", visible: false })
  queue.push(stage)
  queue.resetScene()
  frame()
  assert.deepEqual(sent, [])
  assert.equal(frames.size, 0)
  queue.setReady(true)
  frame()
  await settle()
  assert.equal(state.grid, false)
  assert.deepEqual(state.stage, stage)
  assert.deepEqual(state.motion, [])
  assert.deepEqual(sent, [[{ kind: "grid", visible: false }, stage]])
})

test("ready frames coalesce durable state without reordering motion", async () => {
  const sent: ViewportCommand[][] = []
  const queue = new ViewportInputQueue(
    async (commands) => {
      sent.push(commands)
    },
    (error) => assert.fail(String(error)),
  )
  queue.setReady(true)
  const orbit: ViewportCommand = { kind: "orbit", dx: 2, dy: 3 }
  const pan: ViewportCommand = { kind: "pan", dx: 4, dy: -5 }
  const zoom: ViewportCommand = { kind: "zoom", factor: 0.88 }
  const stage: ViewportCommand = { kind: "stage", left: 0, top: 80, width: 800, height: 500 }
  queue.push({ kind: "grid", visible: true })
  queue.push(orbit)
  queue.push({ kind: "stage", left: 0, top: 0, width: 1, height: 1 })
  queue.push(pan)
  queue.push({ kind: "grid", visible: false })
  queue.push(zoom)
  queue.push(stage)
  frame()
  await settle()
  assert.deepEqual(sent, [[orbit, pan, { kind: "grid", visible: false }, zoom, stage]])
})

test("continuous drag preserves ordered input with only one native frame in flight", async () => {
  const sent: ViewportCommand[][] = []
  const completing: (() => void)[] = []
  const errors: unknown[] = []
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      return new Promise<void>((resolve) => completing.push(resolve))
    },
    (error) => errors.push(error),
  )
  queue.setReady(true)
  const first: ViewportCommand[] = Array.from({ length: 20 }, () => ({
    kind: "orbit",
    dx: 1,
    dy: -1,
  }))
  for (const command of first) queue.push(command)
  frame()
  assert.deepEqual(sent, [first])
  const later: ViewportCommand[] = [
    { kind: "orbit", dx: 0, dy: 400 },
    { kind: "orbit", dx: 0, dy: -5 },
    { kind: "fit" },
    { kind: "pan", dx: 3, dy: 1 },
    { kind: "wheel", deltaY: -120, deltaMode: 0 },
    { kind: "arrow", dx: -1, dy: 0, panning: false },
  ]
  for (const command of later) queue.push(command)
  frame()
  assert.equal(sent.length, 1)
  nextCompletion(completing)()
  await settle()
  frame()
  assert.deepEqual(sent, [first, later])
  nextCompletion(completing)()
  await settle()
  frame()
  assert.equal(sent.length, 2)
  assert.deepEqual(errors, [])
})

test("reset during an in-flight orbit delivers durable updates and drops pending pan", async () => {
  const state = viewportState()
  const completing: (() => void)[] = []
  const queue = new ViewportInputQueue(
    (commands) =>
      new Promise<void>((resolve) =>
        completing.push(() => {
          state.apply(commands)
          resolve()
        }),
      ),
    (error) => assert.fail(String(error)),
  )
  queue.setReady(true)
  const orbit: ViewportCommand = { kind: "orbit", dx: 1, dy: 0 }
  queue.push(orbit)
  frame()
  const stage: ViewportCommand = { kind: "stage", left: 0, top: 80, width: 800, height: 500 }
  queue.push({ kind: "grid", visible: false })
  queue.push(stage)
  queue.push({ kind: "pan", dx: 9, dy: 0 })
  queue.resetScene()
  frame()
  assert.equal(completing.length, 1)
  nextCompletion(completing)()
  await settle()
  frame()
  nextCompletion(completing)()
  await settle()
  assert.equal(state.grid, false)
  assert.deepEqual(state.stage, stage)
  assert.deepEqual(state.motion, [orbit])
})

test("reset replays already delivered durable state but cancels scheduled movement", async () => {
  const state = viewportState()
  const queue = new ViewportInputQueue(
    async (commands) => {
      state.apply(commands)
    },
    (error) => assert.fail(String(error)),
  )
  queue.setReady(true)
  const stage: ViewportCommand = { kind: "stage", left: 10, top: 80, width: 800, height: 500 }
  queue.push({ kind: "grid", visible: false })
  queue.push(stage)
  frame()
  await settle()
  assert.equal(state.grid, false)
  assert.deepEqual(state.stage, stage)
  state.grid = true
  state.stage = null
  queue.push({ kind: "pan", dx: 9, dy: 0 })
  queue.resetScene()
  frame()
  await settle()
  assert.equal(state.grid, false)
  assert.deepEqual(state.stage, stage)
  assert.deepEqual(state.motion, [])
})

test("reset ignores old errors without discarding the new scene's input", async () => {
  const sent: ViewportCommand[][] = []
  let rejectOld: ((error: unknown) => void) | undefined
  const errors: unknown[] = []
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      if (sent.length === 1)
        return new Promise<void>((_, reject) => {
          rejectOld = reject
        })
      return Promise.resolve()
    },
    (error) => errors.push(error),
  )
  queue.setReady(true)
  const orbit: ViewportCommand = { kind: "orbit", dx: 2, dy: 0 }
  queue.push(orbit)
  frame()
  queue.push({ kind: "grid", visible: false })
  queue.push({ kind: "orbit", dx: 100, dy: 0 })
  queue.resetScene()
  const pan: ViewportCommand = { kind: "pan", dx: 3, dy: 1 }
  queue.push(pan)
  assert.ok(rejectOld)
  rejectOld(new Error("Old viewport failed"))
  await settle()
  frame()
  await settle()
  assert.deepEqual(errors, [])
  assert.deepEqual(sent, [[orbit], [{ kind: "grid", visible: false }, pan]])
})

test("pausing invalidates old errors and resumes with only the latest durable values", async () => {
  const state = viewportState()
  const sent: ViewportCommand[][] = []
  const errors: unknown[] = []
  let rejectOld: ((error: unknown) => void) | undefined
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      if (sent.length === 1)
        return new Promise<void>((_, reject) => {
          rejectOld = reject
        })
      state.apply(commands)
      return Promise.resolve()
    },
    (error) => errors.push(error),
  )
  queue.setReady(true)
  const orbit: ViewportCommand = { kind: "orbit", dx: 2, dy: 0 }
  queue.push(orbit)
  frame()
  queue.push({ kind: "pan", dx: 10, dy: 0 })
  queue.setReady(false)
  queue.push({ kind: "grid", visible: true })
  queue.push({ kind: "grid", visible: false })
  const stage: ViewportCommand = { kind: "stage", left: 0, top: 80, width: 900, height: 600 }
  queue.push(stage)
  queue.push({ kind: "fit" })
  assert.ok(rejectOld)
  rejectOld(new Error("Paused viewport failed"))
  await settle()
  frame()
  assert.deepEqual(errors, [])
  assert.deepEqual(sent, [[orbit]])
  queue.setReady(true)
  frame()
  await settle()
  assert.equal(state.grid, false)
  assert.deepEqual(state.stage, stage)
  assert.deepEqual(state.motion, [])
})

test("full clear cancels scheduled frames, forgets durable state and pauses input", async () => {
  const sent: ViewportCommand[][] = []
  const queue = new ViewportInputQueue(
    async (commands) => {
      sent.push(commands)
    },
    (error) => assert.fail(String(error)),
  )
  queue.setReady(true)
  queue.push({ kind: "grid", visible: false })
  queue.push({ kind: "stage", left: 0, top: 80, width: 800, height: 500 })
  queue.push({ kind: "pan", dx: 9, dy: 0 })
  queue.clear()
  assert.equal(frames.size, 0)
  queue.push({ kind: "fit" })
  frame()
  assert.deepEqual(sent, [])
  queue.setReady(true)
  frame()
  assert.deepEqual(sent, [])
  queue.push({ kind: "zoom", factor: 1.12 })
  frame()
  await settle()
  assert.deepEqual(sent, [[{ kind: "zoom", factor: 1.12 }]])
})

test("full clear ignores an in-flight rejection even after readiness returns", async () => {
  const sent: ViewportCommand[][] = []
  const errors: unknown[] = []
  let rejectOld: ((error: unknown) => void) | undefined
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      if (sent.length === 1)
        return new Promise<void>((_, reject) => {
          rejectOld = reject
        })
      return Promise.resolve()
    },
    (error) => errors.push(error),
  )
  queue.setReady(true)
  queue.push({ kind: "grid", visible: false })
  frame()
  queue.clear()
  queue.setReady(true)
  const fit: ViewportCommand = { kind: "fit" }
  queue.push(fit)
  frame()
  assert.deepEqual(sent, [[{ kind: "grid", visible: false }]])
  assert.ok(rejectOld)
  rejectOld(new Error("Unmounted viewport failed"))
  await settle()
  frame()
  await settle()
  assert.deepEqual(errors, [])
  assert.deepEqual(sent, [[{ kind: "grid", visible: false }], [fit]])
})

test("a failed current frame reports the real error without replaying pending input", async () => {
  const rejecting: ((error: unknown) => void)[] = []
  const errors: unknown[] = []
  const queue = new ViewportInputQueue(
    () => new Promise<void>((_, failure) => rejecting.push(failure)),
    (error) => errors.push(error),
  )
  queue.setReady(true)
  queue.push({ kind: "fit" })
  frame()
  queue.push({ kind: "zoom", factor: 0.88 })
  const error = new Error("Graphics surface lost")
  const reject = rejecting.shift()
  assert.ok(reject, "Expected a pending frame rejection")
  reject(error)
  await settle()
  assert.deepEqual(errors, [error])
  assert.equal(frames.size, 0)
})
