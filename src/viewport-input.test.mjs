import assert from "node:assert/strict"
import { afterEach, beforeEach, test, vi } from "vite-plus/test"
import { ViewportInputQueue } from "./viewport-input"

let frames
beforeEach(() => {
  frames = new Map()
  let id = 0
  vi.stubGlobal("requestAnimationFrame", (callback) => {
    frames.set(++id, callback)
    return id
  })
  vi.stubGlobal("cancelAnimationFrame", (id) => frames.delete(id))
})
afterEach(() => vi.unstubAllGlobals())

function frame() {
  const callbacks = [...frames.values()]
  frames.clear()
  for (const callback of callbacks) callback(0)
}
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

test("continuous drag preserves ordered input with only one native frame in flight", async () => {
  const sent = []
  const completing = []
  const errors = []
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      return new Promise((resolve) => completing.push(resolve))
    },
    (error) => errors.push(error),
  )
  const first = Array.from({ length: 20 }, () => ({ kind: "orbit", dx: 1, dy: -1 }))
  for (const command of first) queue.push(command)
  frame()
  assert.deepEqual(sent, [first])
  const later = [
    { kind: "orbit", dx: 0, dy: 400 },
    { kind: "orbit", dx: 0, dy: -5 },
    { kind: "fit" },
    { kind: "pan", dx: 3, dy: 1 },
  ]
  for (const command of later) queue.push(command)
  frame()
  assert.equal(sent.length, 1)
  completing.shift()()
  await settle()
  frame()
  assert.deepEqual(sent, [first, later])
  completing.shift()()
  await settle()
  frame()
  assert.equal(sent.length, 2)
  assert.deepEqual(errors, [])
})

test("leaving a preview drops pending movement and ignores stale errors", async () => {
  let reject
  const sent = []
  const errors = []
  const queue = new ViewportInputQueue(
    (commands) => {
      sent.push(commands)
      return new Promise((_, failure) => {
        reject = failure
      })
    },
    (error) => errors.push(error),
  )
  queue.push({ kind: "orbit", dx: 2, dy: 0 })
  frame()
  queue.push({ kind: "orbit", dx: 100, dy: 0 })
  queue.clear()
  reject(new Error("Old viewport failed"))
  await settle()
  frame()
  assert.equal(sent.length, 1)
  assert.deepEqual(errors, [])
})

test("a failed current frame reports the real error without replaying pending input", async () => {
  let reject
  const errors = []
  const queue = new ViewportInputQueue(
    () =>
      new Promise((_, failure) => {
        reject = failure
      }),
    (error) => errors.push(error),
  )
  queue.push({ kind: "fit" })
  frame()
  queue.push({ kind: "zoom", factor: 0.88 })
  const error = new Error("Graphics surface lost")
  reject(error)
  await settle()
  assert.deepEqual(errors, [error])
  assert.equal(frames.size, 0)
})
