import assert from "node:assert/strict"
import { test } from "vite-plus/test"
import { PreparedUploads } from "./upload-preparation.ts"

const turn = () => new Promise((resolve) => setImmediate(resolve))
const guard = () => {}
function pages(count) {
  return Array.from({ length: count }, (_, index) => ({
    slices: [
      {
        bufferId: 0,
        offset: index * 4,
        length: 4,
        pageOffset: 0,
        target: { kind: "geometry", partIndex: 0, index: false, gpuOffset: index * 4 },
      },
    ],
    byteLength: 4,
    payloadBytes: 4,
  }))
}

for (const [count, speedFirst, limit] of [
  [1, false, 1],
  [8, false, 4],
  [8, true, 8],
]) {
  test(`holds a ${limit}-page read window until GPU consumption releases a slot`, async () => {
    const gates = Array.from({ length: limit + 1 }, () => Promise.withResolvers())
    const reads = []
    const preparation = new PreparedUploads(
      pages(limit + 1).values(),
      async (page) => {
        const index = page.slices[0].offset / 4
        reads.push(index)
        await gates[index].promise
        return Uint8Array.of(index, 0, 0, 0).buffer
      },
      count,
      speedFirst,
      guard,
    )
    await turn()
    assert.deepEqual(
      reads,
      Array.from({ length: limit }, (_, index) => index),
    )
    const first = preparation.take()
    gates[0].resolve()
    assert.equal(new Uint8Array((await first).buffer)[0], 0)
    await turn()
    assert.equal(reads.length, limit)
    preparation.release()
    await turn()
    assert.equal(reads.at(-1), limit)
    for (const gate of gates) gate.resolve()
    const consumed = [0]
    while (true) {
      const prepared = await preparation.take()
      if (!prepared) break
      consumed.push(new Uint8Array(prepared.buffer)[0])
      preparation.release()
    }
    preparation.stop()
    await preparation.drained
    assert.deepEqual(
      consumed,
      Array.from({ length: limit + 1 }, (_, index) => index),
    )
  })
}

test("replacement reads wait until every cancelled generation read has actually drained", async () => {
  const gates = [Promise.withResolvers(), Promise.withResolvers()]
  const previous = new PreparedUploads(
    pages(2).values(),
    async (page) => {
      await gates[page.slices[0].offset / 4].promise
      return new ArrayBuffer(4)
    },
    2,
    true,
    guard,
  )
  const first = previous.take()
  await turn()
  const rejected = assert.rejects(first, /Cancelled/)
  previous.stop()
  await rejected
  const nextReads = []
  const replacement = new PreparedUploads(
    pages(1).values(),
    async (page) => {
      nextReads.push(page.slices[0].offset)
      return Uint8Array.of(42, 0, 0, 0).buffer
    },
    1,
    false,
    guard,
    previous.drained,
  )
  const next = replacement.take()
  gates[0].resolve()
  await turn()
  assert.deepEqual(nextReads, [])
  gates[1].resolve()
  assert.equal(new Uint8Array((await next).buffer)[0], 42)
  assert.deepEqual(nextReads, [0])
  replacement.release()
  assert.equal(await replacement.take(), null)
  replacement.stop()
  await replacement.drained
})

test("a failed or truncated native page prevents its upload and drains outstanding reads", async () => {
  for (const read of [
    async () => {
      throw new Error("native read failed")
    },
    async () => new ArrayBuffer(3),
  ]) {
    const preparation = new PreparedUploads(pages(2).values(), read, 2, false, guard)
    await assert.rejects(preparation.take())
    await preparation.drained
    assert.throws(() => preparation.release())
  }
})
