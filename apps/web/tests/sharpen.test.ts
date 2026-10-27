import { describe, expect, it } from 'vitest'
import { fullFrameBytes, needsSharpen } from '../src/viewer/sharpen.ts'

describe('sharpen', () => {
  const paused = { k: 4, zoom: 0.5, dpr: 1, mode: 'paused', buffering: false } as const

  it('when paused and a block covers more than one device pixel', () => {
    // 4 cells per block at 0.5 px per cell: 2 px per block.
    expect(needsSharpen(paused)).toBe(true)
    // At 0.25 px per cell a block is one pixel: the viewer format shows every detail there is.
    expect(needsSharpen({ ...paused, zoom: 0.25 })).toBe(false)
    // The same zoom on a high-density screen: two device pixels per block.
    expect(needsSharpen({ ...paused, zoom: 0.25, dpr: 2 })).toBe(true)
  })

  it('never while playing, live or waiting for a frame, nor at full resolution already', () => {
    expect(needsSharpen({ ...paused, mode: 'playing' })).toBe(false)
    expect(needsSharpen({ ...paused, mode: 'live' })).toBe(false)
    expect(needsSharpen({ ...paused, buffering: true })).toBe(false)
    expect(needsSharpen({ ...paused, k: 1, zoom: 32 })).toBe(false)
  })

  it('weighs a full-resolution frame in f16', () => {
    // 4096² cells, velocity (2) + pressure + vorticity: 4 components of 2 bytes.
    const fields = [{ components: 2 }, { components: 1 }, { components: 1 }]
    expect(fullFrameBytes(4096, 4096, fields)).toBe(4096 * 4096 * 8)
    expect(fullFrameBytes(442, 84, fields)).toBe(297_024)
  })
})
