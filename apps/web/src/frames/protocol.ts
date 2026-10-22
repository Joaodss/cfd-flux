// The messages between the page and the decode worker, and the worker's handler (kept out of the
// worker file so tests can call it directly, without a Worker).
//
// A worker shares no memory with the page: messages are copied, except `ArrayBuffer`s listed as
// *transferable*, which are moved (the sender's copy becomes empty) — so compressed frames go in
// and decoded fields come back without copying.
import { frameStats, type FrameStats } from '../viewer/stats.ts'
import {
  streamlines,
  toSegments,
  velocityField,
  type StreamlineOptions,
} from '../viewer/streamlines.ts'
import { halfToFloat } from './half.ts'
import { decodeFrame, type DecodedFrame } from './lffr.ts'

export type Request =
  | { id: number; kind: 'decode'; buffer: ArrayBuffer }
  /** Fetches a stored frame (same origin, e.g. `/api/simulations/{id}/frames/{n}?…`) and decodes it. */
  | { id: number; kind: 'fetch'; url: string }
  /**
   * The fluid fraction per block of the frames to come (`null`: every block is fluid), used by
   * their statistics. No answer; later requests see it (messages are handled in order).
   */
  | { id: number; kind: 'mask'; fraction: Float32Array | null }
  /** The run's cells (0 = fluid) for the streamlines, which stop at the others. No answer. */
  | { id: number; kind: 'cells'; cellType: Uint8Array; width: number; height: number }
  /**
   * The streamlines of a velocity field (the `velocity` of a frame, a copy: the page keeps its
   * own) over the blocks of `k × k` cells, as line segments in cells (x0, y0, x1, y1, …).
   */
  | {
      id: number
      kind: 'streamlines'
      velocity: { data: Uint16Array | Float32Array; dtype: 'f16' | 'f32'; k: number }
      options: Partial<StreamlineOptions>
    }

/** A decoded frame and the statistics of its quantities over the fluid. */
export interface Decoded {
  frame: DecodedFrame
  stats: FrameStats
}

export type Response =
  | ({ id: number; ok: true } & Decoded)
  | { id: number; ok: true; segments: Float32Array; lines: number }
  | { id: number; ok: false; error: string; status?: number }

/** The buffers of a decoded frame, to transfer back to the page. */
export function transferables(frame: DecodedFrame): ArrayBuffer[] {
  return frame.fields.map((f) => f.data.buffer as ArrayBuffer)
}

/**
 * The worker's state (the fluid fraction) and its request handler, which never throws (failures
 * become `ok: false`) and answers `null` to requests that need no answer.
 */
export function createHandler(
  fetchImpl: typeof fetch = (input, init) => fetch(input, init),
): (req: Request) => Promise<{ response: Response; transfer: ArrayBuffer[] } | null> {
  let fraction: Float32Array | null = null
  let cells: Extract<Request, { kind: 'cells' }> | null = null
  return async (req) => {
    if (req.kind === 'mask') {
      fraction = req.fraction
      return null
    }
    if (req.kind === 'cells') {
      cells = req
      return null
    }
    if (req.kind === 'streamlines') return traceRequest(req, cells, fraction)
    return decodeRequest(req, fetchImpl, fraction)
  }
}

async function decodeRequest(
  req: Extract<Request, { kind: 'decode' | 'fetch' }>,
  fetchImpl: typeof fetch,
  fraction: Float32Array | null,
): Promise<{ response: Response; transfer: ArrayBuffer[] }> {
  try {
    let buffer: ArrayBuffer
    if (req.kind === 'fetch') {
      const r = await fetchImpl(req.url)
      if (!r.ok) {
        const text = await r.text().catch(() => '')
        let message = `HTTP ${r.status}`
        try {
          message = (JSON.parse(text) as { error: { message: string } }).error.message
        } catch {
          // not the API's error shape: keep the status
        }
        return {
          response: { id: req.id, ok: false, error: message, status: r.status },
          transfer: [],
        }
      }
      buffer = await r.arrayBuffer()
    } else {
      buffer = req.buffer
    }
    const frame = decodeFrame(buffer)
    const stats = frameStats(frame, fraction)
    return { response: { id: req.id, ok: true, frame, stats }, transfer: transferables(frame) }
  } catch (e) {
    return { response: { id: req.id, ok: false, error: (e as Error).message }, transfer: [] }
  }
}

function traceRequest(
  req: Extract<Request, { kind: 'streamlines' }>,
  cells: Extract<Request, { kind: 'cells' }> | null,
  fraction: Float32Array | null,
): { response: Response; transfer: ArrayBuffer[] } {
  try {
    if (!cells) throw new Error('streamlines need the run’s cells first')
    const { data, dtype, k } = req.velocity
    const bw = Math.ceil(cells.width / k)
    const bh = Math.ceil(cells.height / k)
    if (data.length !== 2 * bw * bh) throw new Error('velocity does not match the cells')
    // The frame holds sums over each block's fluid cells divided by the block's cell count: divide
    // by the fluid fraction for the mean over its fluid cells (zero in blocks without fluid).
    const u = new Float32Array(bw * bh)
    const v = new Float32Array(bw * bh)
    for (let b = 0; b < bw * bh; b++) {
      const f = fraction ? fraction[b] : 1
      if (!(f > 0)) continue
      const read = (i: number) => (dtype === 'f16' ? halfToFloat(data[i]) : data[i])
      u[b] = read(2 * b) / f
      v[b] = read(2 * b + 1) / f
    }
    const field = velocityField(cells.width, cells.height, k, u, v, cells.cellType)
    const lines = streamlines(field, req.options)
    const segments = toSegments(lines)
    return {
      response: { id: req.id, ok: true, segments, lines: lines.length },
      transfer: [segments.buffer as ArrayBuffer],
    }
  } catch (e) {
    return { response: { id: req.id, ok: false, error: (e as Error).message }, transfer: [] }
  }
}
