// The messages between the page and the decode worker, and the worker's handler (kept out of the
// worker file so tests can call it directly, without a Worker).
//
// A worker shares no memory with the page: messages are copied, except `ArrayBuffer`s listed as
// *transferable*, which are moved (the sender's copy becomes empty) — so compressed frames go in
// and decoded fields come back without copying.
import { frameStats, type FrameStats } from '../viewer/stats.ts'
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

/** A decoded frame and the statistics of its quantities over the fluid. */
export interface Decoded {
  frame: DecodedFrame
  stats: FrameStats
}

export type Response =
  ({ id: number; ok: true } & Decoded) | { id: number; ok: false; error: string; status?: number }

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
  return async (req) => {
    if (req.kind === 'mask') {
      fraction = req.fraction
      return null
    }
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
