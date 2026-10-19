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

test("leaving a preview drops pending movement and ignores stale errors", async () => {
  const sent: ViewportCommand[][] = []
  const rejecting: ((error: unknown) => void)[] = []
  const errors: unknown[] = []
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      return new Promise<void>((_, failure) => rejecting.push(failure))
    },
    (error) => errors.push(error),
  )
  queue.setReady(true)
  queue.push({ kind: "orbit", dx: 2, dy: 0 })
  frame()
  queue.push({ kind: "orbit", dx: 100, dy: 0 })
  queue.clear()
  const reject = rejecting.shift()
  assert.ok(reject, "Expected a pending frame rejection")
  reject(new Error("Old viewport failed"))
  await settle()
  frame()
  assert.equal(sent.length, 1)
  assert.deepEqual(errors, [])
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
