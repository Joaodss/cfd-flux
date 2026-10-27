// A promise API over the decode workers: each request gets an id, and the worker's answer resolves
// the matching promise (like a TaskCompletionSource per request in .NET).
//
// Frames are spread over a small pool of workers (the next one goes to the least busy), so a
// 1024² frame's decode and statistics in one overlap another's in the next, and streamlines are
// traced in a worker of their own, so a set of them (≈ 0.1 s at 1024²) never holds a frame back.
// Frames may then come back out of order; the viewer looks at each one's index.
import type { Decoded, Request, Response } from './protocol.ts'

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

/**
 * Frame workers by default: about half the machine's threads, less one for the page, from 1 to
 * 4 (a 1024² frame takes ≈ 70 ms in one; four keep up with 30 frames/s, docs/benchmarks.md).
 */
function defaultPool(): number {
  const threads = typeof navigator === 'undefined' ? 4 : navigator.hardwareConcurrency || 4
  return Math.max(1, Math.min(4, Math.floor(threads / 2) - 1))
}

function spawn(): WorkerLike {
  // Vite bundles the worker file from this URL (a module worker, so it can import).
  return new Worker(new URL('./decoder.worker.ts', import.meta.url), {
    type: 'module',
    name: 'frames',
  })
}

export class FrameDecoder {
  /** Every worker; the last one traces the streamlines, the others (or it alone) decode frames. */
  private readonly workers: WorkerLike[]
  private readonly frameWorkers: WorkerLike[]
  private readonly load = new Map<WorkerLike, number>()
  private next = 1
  private readonly pending = new Map<
    number,
    { resolve: (d: never) => void; reject: (e: Error) => void; worker: WorkerLike }
  >()

  /**
   * One worker does everything; with several, the last one traces streamlines and the others
   * decode frames. By default: a pool of frame workers and one for the streamlines.
   */
  constructor(workers?: WorkerLike | WorkerLike[]) {
    this.workers = workers
      ? Array.isArray(workers)
        ? workers
        : [workers]
      : Array.from({ length: defaultPool() + 1 }, spawn)
    this.frameWorkers = this.workers.length > 1 ? this.workers.slice(0, -1) : this.workers
    for (const worker of this.workers) {
      this.load.set(worker, 0)
      worker.addEventListener('message', (e) => {
        const r = e.data
        const p = this.pending.get(r.id)
        if (!p) return
        this.pending.delete(r.id)
        this.load.set(p.worker, this.load.get(p.worker)! - 1)
        if (r.ok) p.resolve(('segments' in r ? r : { frame: r.frame, stats: r.stats }) as never)
        else p.reject(new DecodeError(r.error, r.status))
      })
    }
  }

  private get tracer(): WorkerLike {
    return this.workers[this.workers.length - 1]
  }

  /** Sets the fluid fraction per block used by the statistics of the frames that follow. */
  setMask(fraction: Float32Array | null): void {
    // Messages to a worker are handled in order: every frame asked for later sees it.
    for (const w of this.workers) w.postMessage({ id: 0, kind: 'mask', fraction })
  }

  /** Sets the run's cells (full resolution, 0 = fluid) for the streamlines. */
  setCells(cellType: Uint8Array, width: number, height: number): void {
    this.tracer.postMessage({ id: 0, kind: 'cells', cellType, width, height })
  }

  /**
   * Traces the streamlines of a velocity field (a copy is sent: the page keeps its own) and
   * answers with their line segments in cells.
   */
  streamlines(
    velocity: Extract<Request, { kind: 'streamlines' }>['velocity'],
    options: Extract<Request, { kind: 'streamlines' }>['options'] = {},
  ): Promise<{ segments: Float32Array; lines: number }> {
    return this.request<{ segments: Float32Array; lines: number }>(
      this.tracer,
      { id: 0, kind: 'streamlines', velocity, options },
      [],
    )
  }

  /** Decodes a compressed frame. The buffer is *moved* to the worker: it is empty afterwards. */
  decode(buffer: ArrayBuffer): Promise<Decoded> {
    return this.request<Decoded>(this.idle(), { id: 0, kind: 'decode', buffer }, [buffer])
  }

  /** Fetches and decodes a stored frame in a worker (the bytes never reach this thread). */
  fetch(url: string): Promise<Decoded> {
    return this.request<Decoded>(this.idle(), { id: 0, kind: 'fetch', url }, [])
  }

  /** Frames decoded at the same time (the frame workers): how many to ask for ahead. */
  get parallel(): number {
    return this.frameWorkers.length
  }

  /** Requests still waiting for the workers. */
  get busy(): number {
    return this.pending.size
  }

  dispose(): void {
    for (const w of this.workers) w.terminate()
    for (const p of this.pending.values()) p.reject(new DecodeError('decoder closed'))
    this.pending.clear()
  }

  /** The frame worker with the fewest requests waiting (the first of equals). */
  private idle(): WorkerLike {
    let best = this.frameWorkers[0]
    for (const w of this.frameWorkers) if (this.load.get(w)! < this.load.get(best)!) best = w
    return best
  }

  private request<T>(worker: WorkerLike, req: Request, transfer: Transferable[]): Promise<T> {
    const id = this.next++
    this.load.set(worker, this.load.get(worker)! + 1)
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject, worker })
      worker.postMessage({ ...req, id }, { transfer })
    })
  }
}
