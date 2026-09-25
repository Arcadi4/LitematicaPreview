import { validateUpload } from "./upload-preparation"
import type { UploadPage } from "./upload-layout"

type PrepareMessage = { buffer: ArrayBuffer; page: UploadPage }

self.onmessage = (event: MessageEvent<PrepareMessage>) => {
  const { buffer, page } = event.data
  try {
    validateUpload(buffer, page)
    self.postMessage({ buffer }, { transfer: [buffer] })
  } catch (error) {
    self.postMessage({ error: error instanceof Error ? error.message : String(error) })
  }
}
