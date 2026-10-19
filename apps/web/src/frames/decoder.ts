// A promise API over the decode worker: each request gets an id, and the worker's answer resolves
// the matching promise (like a TaskCompletionSource per request in .NET).
import type { DecodedFrame } from './lffr.ts'
import type { Request, Response } from './protocol.ts'

/** What the decoder needs of a Worker (a stand-in in tests). */
export interface WorkerLike {
  postMessage(message: Request, options?: { transfer?: Transferable[] }): void
  addEventListener(type: 'message', listener: (e: MessageEvent<Response>) => void): void
  terminate(): void
}

/** A frame the worker could not fetch or decode. */
export class DecodeError extends Error {
  /** The HTTP status of a failed fetch (404: no such stored frame). */
  readonly status?: number
  constructor(message: string, status?: number) {
    super(message)
    this.status = status
  }
}

export class FrameDecoder {
  private readonly worker: WorkerLike
  private next = 1
  private readonly pending = new Map<
    number,
    { resolve: (f: DecodedFrame) => void; reject: (e: Error) => void }
  >()

  constructor(worker?: WorkerLike) {
    // Vite bundles the worker file from this URL (a module worker, so it can import).
    this.worker =
      worker ??
      new Worker(new URL('./decoder.worker.ts', import.meta.url), {
        type: 'module',
        name: 'frames',
      })
    this.worker.addEventListener('message', (e) => {
      const r = e.data
      const p = this.pending.get(r.id)
      if (!p) return
      this.pending.delete(r.id)
      if (r.ok) p.resolve(r.frame)
      else p.reject(new DecodeError(r.error, r.status))
    })
  }

  /** Decodes a compressed frame. The buffer is *moved* to the worker: it is empty afterwards. */
  decode(buffer: ArrayBuffer): Promise<DecodedFrame> {
    return this.request({ id: 0, kind: 'decode', buffer }, [buffer])
  }

  /** Fetches and decodes a stored frame in the worker (the bytes never reach this thread). */
  fetch(url: string): Promise<DecodedFrame> {
    return this.request({ id: 0, kind: 'fetch', url }, [])
  }

  /** Requests still waiting for the worker. */
  get busy(): number {
    return this.pending.size
  }

  dispose(): void {
    this.worker.terminate()
    for (const p of this.pending.values()) p.reject(new DecodeError('decoder closed'))
    this.pending.clear()
  }

  private request(req: Request, transfer: Transferable[]): Promise<DecodedFrame> {
    const id = this.next++
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      this.worker.postMessage({ ...req, id }, { transfer })
    })
  }
}
