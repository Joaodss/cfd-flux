// The messages between the page and the decode worker, and the worker's handler (kept out of the
// worker file so tests can call it directly, without a Worker).
//
// A worker shares no memory with the page: messages are copied, except `ArrayBuffer`s listed as
// *transferable*, which are moved (the sender's copy becomes empty) — so compressed frames go in
// and decoded fields come back without copying.
import { decodeFrame, type DecodedFrame } from './lffr.ts'

export type Request =
  | { id: number; kind: 'decode'; buffer: ArrayBuffer }
  /** Fetches a stored frame (same origin, e.g. `/api/simulations/{id}/frames/{n}?…`) and decodes it. */
  | { id: number; kind: 'fetch'; url: string }

export type Response =
  | { id: number; ok: true; frame: DecodedFrame }
  | { id: number; ok: false; error: string; status?: number }

/** The buffers of a decoded frame, to transfer back to the page. */
export function transferables(frame: DecodedFrame): ArrayBuffer[] {
  return frame.fields.map((f) => f.data.buffer as ArrayBuffer)
}

/** Handles one request; never throws (failures become `ok: false`). */
export async function handle(
  req: Request,
  fetchImpl: typeof fetch = fetch,
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
    return { response: { id: req.id, ok: true, frame }, transfer: transferables(frame) }
  } catch (e) {
    return { response: { id: req.id, ok: false, error: (e as Error).message }, transfer: [] }
  }
}
