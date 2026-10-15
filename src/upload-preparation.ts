import { UPLOAD_CHUNK, type UploadPage } from "./upload-layout"
const PREPARATION_BYTES = 4 * UPLOAD_CHUNK

export type PreparedPage = { page: UploadPage; buffer: ArrayBuffer }

type Result = { buffer: ArrayBuffer } | { error: Error }
type Slot = {
  page: UploadPage
  result: Promise<Result>
  finish: (result: Result) => void
}

const errorOf = (error: unknown): Error =>
  error instanceof Error ? error : new Error(String(error))

// The window includes reads, ready results and the buffer
// currently being uploaded. A slot is not reused until GL has consumed its bytes.
export class PreparedUploads {
  readonly drained: Promise<void>
  private readonly slots: Slot[] = []
  private readonly started: Promise<void>
  private readonly finishDrain: () => void
  private readonly cancelled: Promise<void>
  private readonly finishCancellation: () => void
  private reads = 0
  private failure: Error | null = null
  private ended = false
  private held: Slot | null = null
  private readonly pages: Iterator<UploadPage>
  private readonly read: (page: UploadPage) => Promise<ArrayBuffer>
  private readonly guard: () => void

  constructor(
    pages: Iterator<UploadPage>,
    read: (page: UploadPage) => Promise<ArrayBuffer>,
    count: number,
    speedFirst: boolean,
    guard: () => void,
    preceding: Promise<void> = Promise.resolve(),
  ) {
    if (!Number.isInteger(count) || count < 1 || count > 8)
      throw new Error("Invalid preview read window.")
    this.pages = pages
    this.read = read
    this.guard = guard
    let finishDrain!: () => void
    const readsDrained = new Promise<void>((resolve) => {
      finishDrain = resolve
    })
    this.finishDrain = finishDrain
    let finishCancellation!: () => void
    this.cancelled = new Promise<void>((resolve) => {
      finishCancellation = resolve
    })
    this.finishCancellation = finishCancellation
    // A superseded generation cannot let its successor pass earlier live reads.
    this.drained = preceding.then(() => readsDrained)
    this.started = preceding.then(() => {
      if (this.failure) return
      try {
        guard()
        const limit = speedFirst ? count : Math.min(count, PREPARATION_BYTES / UPLOAD_CHUNK)
        for (let index = 0; index < limit && !this.failure; index++) {
          const next = this.pages.next()
          if (next.done) {
            this.ended = true
            break
          }
          this.schedule(next.value)
        }
      } catch (error) {
        this.stop(errorOf(error))
      }
    })
  }

  async take(): Promise<PreparedPage | null> {
    await Promise.race([this.started, this.cancelled])
    if (this.failure) throw this.failure
    this.guard()
    if (this.held) throw new Error("The previous preview page has not been uploaded.")
    const slot = this.slots[0]
    if (!slot) {
      if (this.ended) return null
      throw new Error("The preview upload order is inconsistent.")
    }
    const result = await slot.result
    if (this.failure) throw this.failure
    this.guard()
    if ("error" in result) throw result.error
    this.held = slot
    return { page: slot.page, buffer: result.buffer }
  }

  release(): void {
    if (this.failure) throw this.failure
    this.guard()
    const slot = this.held
    if (!slot) throw new Error("No preview page is being uploaded.")
    this.held = null
    this.slots.shift()
    if (!this.ended) {
      try {
        const next = this.pages.next()
        if (next.done) this.ended = true
        else this.schedule(next.value)
      } catch (error) {
        this.stop(errorOf(error))
        throw this.failure
      }
    }
  }

  stop(error = new Error("Cancelled")): void {
    if (this.failure) return
    this.failure = error
    this.finishCancellation()
    for (const slot of this.slots) slot.finish({ error })
    this.slots.length = 0
    this.held = null
    if (this.reads === 0) this.finishDrain()
  }

  private schedule(page: UploadPage): void {
    this.guard()
    if (
      !Number.isSafeInteger(page.byteLength) ||
      page.byteLength <= 0 ||
      page.byteLength > UPLOAD_CHUNK
    )
      throw new Error("Invalid preview preparation page.")
    let finish!: (result: Result) => void
    const result = new Promise<Result>((resolve) => {
      finish = resolve
    })
    const slot = { page, result, finish }
    this.slots.push(slot)
    this.reads++
    void this.prepare(slot)
  }

  private async prepare(slot: Slot): Promise<void> {
    try {
      const buffer = await this.read(slot.page)
      if (this.failure) return
      this.guard()
      if (!(buffer instanceof ArrayBuffer) || buffer.byteLength !== slot.page.byteLength)
        throw new Error("The preview contains an incomplete data page.")
      slot.finish({ buffer })
    } catch (error) {
      this.stop(errorOf(error))
    } finally {
      this.reads--
      if (this.failure && this.reads === 0) this.finishDrain()
    }
  }
}
