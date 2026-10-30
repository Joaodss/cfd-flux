import { describe, expect, it } from 'vitest'
import type { RunMeta } from '../src/api/jobs.ts'
import type { DecodedFrame } from '../src/frames/lffr.ts'
import { annotatedSize, FOOTER_HEIGHT, footerText } from '../src/viewer/export/annotate.ts'
import {
  coefficientsCsv,
  frameCsv,
  num,
  nusseltCsv,
  runFiles,
  statisticsCsv,
} from '../src/viewer/export/csv.ts'
import {
  bitrateFor,
  chooseCodec,
  defaultScale,
  recorderFrames,
  videoTiming,
} from '../src/viewer/export/video.ts'
import type { Table } from '../src/viewer/series.ts'
import type { ForceStatistics } from '../src/viewer/statistics.ts'

const meta = {
  units: { dx: 0.01, dt: 0.001, rho0: 1000 },
  dimensionless: { reference: { length: 0.2, velocity: 1, reynolds: 100 } },
  run: { outputEverySteps: 10, sampleEverySteps: 1, endTime: 1 },
} as RunMeta

describe('CSV exports', () => {
  it('formats numbers briefly and gaps as NaN', () => {
    expect(num(1 / 3)).toBe('0.3333333')
    expect(num(1234567.891)).toBe('1234568')
    expect(num(null)).toBe('NaN')
    expect(num(Infinity)).toBe('NaN')
  })

  it('writes the coefficients of one element from its forces', () => {
    // C = 2 F / (ρ U² L) = F / 100 here.
    const forces: Table = {
      columns: ['step', 'time_s', 'Cylinder.fx_N_m', 'Cylinder.fy_N_m'],
      data: [
        [0, 10],
        [0, 0.01],
        [100, 200],
        [null, -50],
      ],
    }
    expect(coefficientsCsv(forces, 'Cylinder', meta)).toBe(
      'step,time_s,cd,cl\n0,0,1,NaN\n10,0.01,2,-0.5\n',
    )
    expect(coefficientsCsv(forces, 'Nothing', meta)).toBeNull()
  })

  it('writes the Nusselt number of every thermal wall per row', () => {
    // Nu = (q / length) L / (k ΔT) = 2 q here.
    const heat: Table = {
      columns: ['step', 'time_s', 'Hot.q_W_m', 'Hot.length_m', 'Cold.q_W_m', 'Cold.length_m'],
      data: [
        [0, 10],
        [0, 0.01],
        [1, 1.5],
        [0.5, 0.5],
        [-1, null],
        [0.5, 0.5],
      ],
    }
    const scales = { length: 0.1, conductivity: 0.1, deltaT: 1 }
    expect(nusseltCsv(heat, scales)).toBe('step,time_s,Hot.nu,Cold.nu\n0,0,2,-2\n10,0.01,3,NaN\n')
    expect(nusseltCsv({ columns: ['step', 'time_s'], data: [[], []] }, scales)).toBeNull()
  })

  it('offers heat.csv only for runs that have one', () => {
    expect(runFiles(meta)).not.toContain('heat.csv')
    expect(runFiles(null)).not.toContain('heat.csv')
    expect(runFiles({ ...meta, run: { ...meta.run, heat: true } })).toEqual([
      'forces.csv',
      'probes.csv',
      'diagnostics.csv',
      'heat.csv',
      'meta.json',
      'scene.json',
    ])
  })

  it('writes the window statistics with the element and window', () => {
    const stats: ForceStatistics = {
      window: { from: 10, to: 20 },
      samples: 500,
      cdMean: 3.2,
      cdMax: 3.375,
      clMean: 0,
      clAmplitude: 1,
      clMax: 1.04,
      frequency: 1.5,
      strouhal: 0.3,
      periods: 15,
    }
    const lines = statisticsCsv(stats, 'Cylinder').trim().split('\n')
    expect(lines.slice(0, 5)).toEqual([
      'name,value',
      'element,Cylinder',
      'window_from_s,10',
      'window_to_s,20',
      'samples,500',
    ])
    expect(lines).toContain('strouhal,0.3')
    expect(lines).toContain('cd_max,3.375')
  })

  it('writes a frame per block, as means over the fluid, with solid blocks empty', () => {
    // 3 × 1 blocks of 2 cells in a 5 × 2 domain (the last block is one cell wide); cell size 0.5 m.
    const frame: DecodedFrame = {
      header: {
        jobId: 'j',
        frameIndex: 3,
        step: 30,
        simTime: 0.3,
        width: 3,
        height: 1,
        codec: 'none',
        fields: [
          { name: 'velocity', components: 2, dtype: 'f32' },
          { name: 'pressure', components: 1, dtype: 'f32' },
        ],
      },
      fields: [
        {
          name: 'velocity',
          components: 2,
          dtype: 'f32',
          data: Float32Array.from([0.5, 0, 1, 2, 0, 0]),
        },
        { name: 'pressure', components: 1, dtype: 'f32', data: Float32Array.from([10, 4, 0]) },
      ],
    }
    // The first block is half solid: its sums are divided by 0.5. The last one is all solid.
    const csv = frameCsv(
      frame,
      Float32Array.from([0.5, 1, 0]),
      { width: 5, height: 2, cellSize: 0.5 },
      2,
    )
    expect(csv.trim().split('\n')).toEqual([
      'x_m,y_m,velocity_x,velocity_y,pressure',
      '0.5,0.5,1,0,20',
      '1.5,0.5,1,2,4',
      '2.25,0.5,,,',
    ])
  })
})

describe('annotations', () => {
  it('words the footer and adds its height', () => {
    const t = footerText({
      lut: new Uint8Array(1024),
      range: [0, 2],
      unit: 'm/s',
      label: 'Velocity |u|',
      time: 1.25,
      frame: 62,
      step: 12500,
    })
    expect(t.left).toBe('Velocity |u| (m/s)')
    expect(t.right).toBe('t = 1.25 s · frame 62 · step 12,500')
    expect(t.ticks).toHaveLength(5)
    expect(annotatedSize(442, 84)).toEqual({ width: 442, height: 84 + FOOTER_HEIGHT })
    expect(annotatedSize(100, 50).width).toBe(360)
  })
})

describe('video codec', () => {
  it('takes the first codec the platform supports at the size', async () => {
    const only = (prefix: string) => async (c: VideoEncoderConfig) => c.codec.startsWith(prefix)
    expect((await chooseCodec(884, 168, 30, 1e6, only('avc')))?.codec).toBe('avc')
    expect((await chooseCodec(884, 168, 30, 1e6, only('vp09')))?.codec).toBe('vp9')
    expect((await chooseCodec(884, 168, 30, 1e6, only('av01')))?.codec).toBe('av1')
    expect(await chooseCodec(884, 168, 30, 1e6, async () => false)).toBeNull()
    // A bigger level for frames beyond 2048 × 1152.
    expect((await chooseCodec(4000, 2000, 30, 1e6, only('avc')))?.string).toBe('avc1.640033')
  })

  it('picks a bit rate per pixel by quality, between 2 and 50 Mbit/s', () => {
    expect(bitrateFor(100, 100, 30, 'high')).toBe(2e6)
    // The cylinder at 2 px per cell with its footer, 50 frames/s.
    expect(bitrateFor(884, 216, 50, 'high')).toBe(19094400)
    expect(bitrateFor(884, 216, 50, 'standard')).toBe(4773600)
    expect(bitrateFor(7680, 4320, 60, 'standard')).toBe(50e6)
  })

  it('follows the flow’s clock: real time or × N, skipping frames past the cap', () => {
    // Frames every 0.02 s: real time is 50 frames/s.
    expect(videoTiming(0.02, 1, 60)).toEqual({ fps: 50, step: 1, flowPerSecond: 1 })
    // × 2 would be 100 frames/s: every 2nd frame at 50.
    expect(videoTiming(0.02, 2, 60)).toEqual({ fps: 50, step: 2, flowPerSecond: 2 })
    // × 5 at most 30 frames/s: 250 → every 9th frame, 27.8 frames/s.
    const fast = videoTiming(0.02, 5, 30)
    expect(fast.step).toBe(9)
    expect(fast.fps).toBeCloseTo(250 / 9)
    // Slow motion of sparse frames: few frames per second, none skipped.
    expect(videoTiming(0.5, 0.25, 60)).toEqual({ fps: 0.5, step: 1, flowPerSecond: 0.25 })
    // Frame times rounded on the way (0.02 s as 0.01999…) still give 50.
    expect(videoTiming(0.019990224828934505, 1, 60).fps).toBe(50)
    expect(videoTiming(0.03, 1, 60).fps).toBeCloseTo(33.33, 2)
    // An exact multiple is not rounded up to the next step.
    expect(videoTiming(0.01, 1.2, 60).step).toBe(2)
    // Interval unknown: one frame per output frame at the cap.
    expect(videoTiming(null, 1, 30)).toEqual({ fps: 30, step: 1, flowPerSecond: null })
  })

  it('sizes the video to fit 1920 × 1080 by default', () => {
    expect(defaultScale(442, 84)).toBe(4)
    expect(defaultScale(512, 513)).toBe(2)
    expect(defaultScale(1023, 1024)).toBe(1)
    expect(defaultScale(4000, 4001)).toBe(1)
  })

  it('lets the WebM recorder draw ahead within 128 MiB, at least 2 frames', () => {
    expect(recorderFrames(1022, 1072)).toBe(30)
    expect(recorderFrames(1768, 384)).toBe(49)
    expect(recorderFrames(8192, 8192)).toBe(2)
  })
})
