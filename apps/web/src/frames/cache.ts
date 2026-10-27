// Decoded frames kept in memory, by output index, up to a byte budget (design proposal §3.2).
// The server stores every frame, so evicting one only means fetching it again: frames farthest
// from the playhead go first.
import type { Decoded } from './protocol.ts'

/** Default budget: 512 MiB of decoded frames. */
export const DEFAULT_BUDGET = 512 * 2 ** 20

export function frameBytes(d: Decoded): number {
  return d.frame.fields.reduce((n, f) => n + f.data.byteLength, 0)
}

export class FrameCache {
  private readonly frames = new Map<number, Decoded>()
  private total = 0
  private limit: number

  constructor(budget = DEFAULT_BUDGET) {
    this.limit = budget
  }

  /** Bytes of decoded frames held at most. */
  get budget(): number {
    return this.limit
  }

  /** A new budget (a setting): over it, the frames farthest from `playhead` go at once. */
  setBudget(budget: number, playhead: number): void {
    this.limit = budget
    this.evict(playhead, playhead)
  }

  get(index: number): Decoded | undefined {
    return this.frames.get(index)
  }

  has(index: number): boolean {
    return this.frames.has(index)
  }

  get size(): number {
    return this.frames.size
  }

  get bytes(): number {
    return this.total
  }

  /** Adds (or replaces) a frame, then evicts the frames farthest from `playhead` over budget. */
  put(index: number, frame: Decoded, playhead: number): void {
    const old = this.frames.get(index)
    if (old) this.total -= frameBytes(old)
    this.frames.set(index, frame)
    this.total += frameBytes(frame)
    this.evict(playhead, index)
  }

  /** Evicts the frames farthest from `playhead` (never the playhead's) until within budget. */
  private evict(playhead: number, fallback: number): void {
    while (this.total > this.limit && this.frames.size > 1) {
      let far = fallback
      let distance = -1
      for (const i of this.frames.keys()) {
        const d = Math.abs(i - playhead)
        if (i !== playhead && d > distance) {
          far = i
          distance = d
        }
      }
      this.total -= frameBytes(this.frames.get(far)!)
      this.frames.delete(far)
    }
  }

  clear(): void {
    this.frames.clear()
    this.total = 0
  }
}
