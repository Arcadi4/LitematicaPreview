import assert from "node:assert/strict"
import { afterEach, beforeEach, test, vi } from "vite-plus/test"
import {
  FlyMotion,
  flyDirection,
  flySeconds,
  isFlyKey,
  maxFlyStep,
  type FlyDirection,
} from "./fly-controls"

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

function frame(time: number) {
  const callbacks = [...frames.values()]
  frames.clear()
  for (const callback of callbacks) callback(time)
}

function flyMotion(clock: { now: number }) {
  const moves: Array<FlyDirection & { seconds: number }> = []
  const motion = new FlyMotion(
    (direction, seconds) => moves.push({ ...direction, seconds }),
    () => clock.now,
  )
  return { motion, moves }
}

test("fly keys map to camera-relative axes with inverses cancelling", () => {
  assert.deepEqual(flyDirection(new Set()), { forward: 0, right: 0, up: 0 })
  assert.deepEqual(flyDirection(new Set(["KeyW", "KeyD", "Space"])), {
    forward: 1,
    right: 1,
    up: 1,
  })
  assert.deepEqual(flyDirection(new Set(["KeyS", "KeyA", "ControlLeft"])), {
    forward: -1,
    right: -1,
    up: -1,
  })
  assert.deepEqual(flyDirection(new Set(["KeyW", "KeyS"])), { forward: 0, right: 0, up: 0 })
})

test("both control keys still clamp the vertical axis to one unit", () => {
  assert.deepEqual(flyDirection(new Set(["ControlLeft", "ControlRight"])), {
    forward: 0,
    right: 0,
    up: -1,
  })
})

test("isFlyKey accepts only movement codes", () => {
  for (const code of ["KeyW", "KeyA", "KeyS", "KeyD", "Space", "ControlLeft", "ControlRight"])
    assert.ok(isFlyKey(code), code)
  for (const code of ["KeyV", "Escape", "ShiftLeft", "KeyQ"]) assert.ok(!isFlyKey(code), code)
})

test("flySeconds scales to seconds and caps a stalled frame", () => {
  assert.equal(flySeconds(1000, 1000), 0)
  assert.equal(flySeconds(900, 1000), 0)
  assert.equal(flySeconds(1050, 1000), 0.05)
  assert.equal(flySeconds(5000, 1000), maxFlyStep)
})

test("a press starts one frame loop that emits elapsed-time moves", () => {
  const clock = { now: 1000 }
  const { motion, moves } = flyMotion(clock)

  assert.equal(motion.press("KeyQ"), false)
  assert.equal(frames.size, 0)

  assert.equal(motion.press("KeyW"), true)
  assert.equal(frames.size, 1)
  frame(1016)
  assert.deepEqual(moves, [{ forward: 1, right: 0, up: 0, seconds: 0.016 }])
  assert.equal(frames.size, 1)

  frame(1032)
  assert.deepEqual(moves[1], { forward: 1, right: 0, up: 0, seconds: 0.016 })
})

test("opposing keys emit no move until one is released", () => {
  const clock = { now: 0 }
  const { motion, moves } = flyMotion(clock)
  motion.press("KeyW")
  motion.press("KeyS")

  frame(16)
  assert.deepEqual(moves, [])
  assert.equal(frames.size, 1)

  motion.release("KeyW")
  frame(32)
  assert.deepEqual(moves, [{ forward: -1, right: 0, up: 0, seconds: 0.016 }])
})

test("releasing the last held key stops the loop and cancels a pending frame", () => {
  const clock = { now: 0 }
  const { motion, moves } = flyMotion(clock)
  motion.press("KeyD")
  motion.release("KeyA")
  assert.equal(frames.size, 1)
  motion.release("KeyD")
  assert.equal(frames.size, 0)
  assert.deepEqual(moves, [])
})

test("stop cancels a pending frame without emitting movement", () => {
  const clock = { now: 0 }
  const { motion, moves } = flyMotion(clock)
  motion.press("Space")
  motion.stop()
  assert.equal(frames.size, 0)
  motion.release("Space")
  assert.deepEqual(moves, [])
})
