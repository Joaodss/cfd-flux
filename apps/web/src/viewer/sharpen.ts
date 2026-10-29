// "Sharpen" (design proposal §3.2, full resolution on demand): when a paused frame is zoomed in
// past the viewer format — one downsampled block covers more than one screen pixel — the viewer
// fetches that frame at full resolution and draws it instead. Not a second cache: one frame, the
// playhead's, dropped as soon as the view or the playhead moves on.
import type { FieldDesc } from '../frames/lffr.ts'
import type { PlayMode } from './timeline.ts'

/** What the viewer shows: off, fetching, the full-resolution frame, or why it was not fetched. */
export type SharpenState =
  | { state: 'off' }
  | { state: 'loading' }
  | { state: 'on'; width: number; height: number }
  | { state: 'skipped'; note: string }

/**
 * Whether the playhead's frame should be shown at full resolution: paused (not playing, not
 * following a live run, not waiting for a frame), a viewer format coarser than the grid (`k` > 1
 * cells per block side), and a block wider than one device pixel at this zoom (CSS pixels per
 * cell × device-pixel ratio).
 */
export function needsSharpen(o: {
  k: number
  zoom: number
  dpr: number
  mode: PlayMode
  buffering: boolean
}): boolean {
  return o.mode === 'paused' && !o.buffering && o.k > 1 && o.k * o.zoom * o.dpr > 1
}

/** Bytes of a frame of these fields at full resolution in f16 (what the guard compares). */
export function fullFrameBytes(
  width: number,
  height: number,
  fields: Pick<FieldDesc, 'components'>[],
): number {
  return width * height * 2 * fields.reduce((n, f) => n + f.components, 0)
}
