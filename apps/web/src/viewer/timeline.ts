// The timeline's rules (design proposal §6), as pure functions the Viewer applies.
//
// Modes: `live` follows the newest frame while the run goes on; `paused` holds a frame;
// `playing` advances through the stored frames at `speed` frames per second.

export type PlayMode = 'live' | 'paused' | 'playing'

export interface Timeline {
  mode: PlayMode
  /** Newest output index known (−1: none yet). */
  last: number
  /** The frame shown or being fetched (null before the first). */
  index: number | null
  /** Frames per second while playing. */
  speed: number
  loop: boolean
  /** Waiting for a frame from the server. */
  buffering: boolean
  /** Why a frame cannot be shown (e.g. frames were not stored). */
  note: string | null
}

export const INITIAL_TIMELINE: Timeline = {
  mode: 'live',
  last: -1,
  index: null,
  speed: 30,
  loop: false,
  buffering: false,
  note: null,
}

export const SPEEDS = [1, 5, 10, 15, 30, 60] as const

/**
 * Where playback goes after `elapsed` seconds from fractional position `position`: the next
 * position, and the mode (past the newest frame: live again while the run goes on, else back to
 * the start when looping, else paused at the end).
 */
export function advance(
  position: number,
  elapsed: number,
  t: Pick<Timeline, 'speed' | 'last' | 'loop'>,
  running: boolean,
): { position: number; mode: PlayMode } {
  const next = position + elapsed * t.speed
  if (Math.floor(next) <= t.last) return { position: next, mode: 'playing' }
  if (running) return { position: t.last, mode: 'live' }
  if (t.loop && t.last > 0) return { position: 0, mode: 'playing' }
  return { position: t.last, mode: 'paused' }
}

/** The frames to fetch ahead of `index` while playing (the next few, within the known frames). */
export function prefetch(index: number, last: number, count = 3): number[] {
  const out: number[] = []
  for (let i = index + 1; i <= Math.min(last, index + count); i++) out.push(i)
  return out
}
