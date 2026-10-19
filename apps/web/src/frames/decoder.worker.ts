// The decode worker: a background thread that turns compressed LFFR frames into typed arrays,
// so a 1024² frame never blocks the page's thread. Started by `FrameDecoder` (decoder.ts).
import { handle, type Request } from './protocol.ts'

self.addEventListener('message', (e: MessageEvent<Request>) => {
  void handle(e.data).then(({ response, transfer }) => self.postMessage(response, { transfer }))
})
