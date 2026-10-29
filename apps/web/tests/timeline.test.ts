import { describe, expect, it } from 'vitest'
import { FrameCache, frameBytes } from '../src/frames/cache.ts'
import type { Decoded } from '../src/frames/protocol.ts'
import { advance, prefetch } from '../src/viewer/timeline.ts'

/** A decoded frame of `bytes` bytes (one f16 field). */
function frame(index: number, bytes = 100): Decoded {
  return {
    frame: {
      header: {
        jobId: 'j',
        frameIndex: index,
        step: index,
        simTime: index,
        width: bytes / 2,
        height: 1,
        codec: 'none',
        fields: [{ name: 'pressure', components: 1, dtype: 'f16' }],
      },
      fields: [{ name: 'pressure', components: 1, dtype: 'f16', data: new Uint16Array(bytes / 2) }],
    },
    stats: {},
  }
}

describe('FrameCache', () => {
  it('evicts the frames farthest from the playhead over its budget', () => {
    const c = new FrameCache(300)
    for (const i of [0, 1, 2]) c.put(i, frame(i), 1)
    expect([c.size, c.bytes]).toEqual([3, 300])
    c.put(3, frame(3), 1) // over budget: frame 3 is the farthest from 1
    expect([c.has(0), c.has(1), c.has(2), c.has(3)]).toEqual([true, true, true, false])
    c.put(3, frame(3), 3) // the playhead moved: frame 0 goes
    expect([c.has(0), c.has(3)]).toEqual([false, true])
    expect(c.bytes).toBe(300)
  })

  it('replaces a frame of the same index (a snapshot) and never drops the only one', () => {
    const c = new FrameCache(150)
    c.put(5, frame(5), 5)
    c.put(5, frame(5, 120), 5)
    expect([c.size, c.bytes]).toEqual([1, 120])
    const big = frame(6, 400)
    c.put(6, big, 6)
    expect([c.size, c.get(6)]).toEqual([1, big])
    expect(frameBytes(big)).toBe(400)
    c.clear()
    expect([c.size, c.bytes]).toEqual([0, 0])
  })

  it('evicts at once when the budget shrinks, farthest from the playhead first', () => {
    const c = new FrameCache(1000)
    for (const i of [0, 1, 2, 3, 4]) c.put(i, frame(i), 0)
    expect(c.size).toBe(5)
    c.setBudget(250, 3)
    // 0 and 1 are the farthest; of 2 and 4 (as far), the older entry goes.
    expect([0, 1, 2, 3, 4].filter((i) => c.has(i))).toEqual([3, 4])
    expect([c.budget, c.bytes]).toEqual([250, 200])
    c.setBudget(1000, 3) // a larger budget keeps what is there
    expect(c.size).toBe(2)
    c.setBudget(10, 3) // never the playhead's frame
    expect([c.size, c.has(3)]).toEqual([1, true])
  })
})

describe('timeline rules', () => {
  const t = { speed: 10, last: 20, loop: false }
  it('advance at the playback speed', () => {
    expect(advance(3, 0.5, t, false)).toEqual({ position: 8, mode: 'playing' })
  })

  it('past the newest frame: live while running, else loop or stop', () => {
    expect(advance(19, 1, t, true)).toEqual({ position: 20, mode: 'live' })
    expect(advance(19, 1, t, false)).toEqual({ position: 20, mode: 'paused' })
    expect(advance(19, 1, { ...t, loop: true }, false)).toEqual({ position: 0, mode: 'playing' })
  })

  it('prefetch the next frames within the known ones', () => {
    expect(prefetch(5, 20)).toEqual([6, 7, 8])
    expect(prefetch(19, 20)).toEqual([20])
    expect(prefetch(20, 20)).toEqual([])
    // An export of every 3rd frame up to 20.
    expect(prefetch(5, 20, 4, 3)).toEqual([8, 11, 14, 17])
    expect(prefetch(14, 20, 4, 3)).toEqual([17, 20])
  })
})
