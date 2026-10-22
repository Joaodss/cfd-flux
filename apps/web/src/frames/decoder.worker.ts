// The decode worker: a background thread that turns compressed LFFR frames into typed arrays,
// so a 1024² frame never blocks the page's thread. Started by `FrameDecoder` (decoder.ts).
import { createHandler, type Request } from './protocol.ts'

const handle = createHandler()
self.addEventListener('message', (e: MessageEvent<Request>) => {
  void handle(e.data).then((r) => r && self.postMessage(r.response, { transfer: r.transfer }))
})
