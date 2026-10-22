import { describe, expect, it } from 'vitest'
import type { RunMeta, StreamMessage } from '../src/api/jobs.ts'
import {
  coefficientFactor,
  diagnosticPlots,
  forceElements,
  forcePlots,
  frameAt,
  probePlots,
} from '../src/viewer/plots.ts'
import { bodies, columnName, shownElement } from '../src/viewer/elements.ts'
import { parseCsv, SeriesBuffer } from '../src/viewer/series.ts'
import { SeriesFeed } from '../src/viewer/seriesFeed.ts'

const FORCES = 'step,time_s,edges.fx_N_m,edges.fy_N_m,cylinder.fx_N_m,cylinder.fy_N_m\n'
const row = (step: number) => [step, step / 1000, 0, 0, 0.6 * step, -0.1 * step]
const line = (step: number) => row(step).join(',') + '\n'

const META: RunMeta = {
  units: { dx: 0.005, dt: 0.001, rho0: 1.2 },
  dimensionless: {
    reference: { length: 0.1, velocity: 1, reynolds: 100, temperatureDifference: 1 },
  },
  run: { outputEverySteps: 10, sampleEverySteps: 1, endTime: 1 },
}

describe('parseCsv', () => {
  it('keeps complete lines only and reads NaN as null', () => {
    expect(parseCsv('')).toBeNull()
    expect(parseCsv('step,time_s')).toBeNull() // the header is still being written
    const p = parseCsv('step,time_s,a.T_K\n0,0,NaN\n1,0.5,2\n2,1,3')
    expect(p?.columns).toEqual(['step', 'time_s', 'a.T_K'])
    expect(p?.rows).toEqual([
      [0, 0, null],
      [1, 0.5, 2],
    ])
  })
})

describe('SeriesBuffer', () => {
  it('appends stream rows after the rows of the file, whichever comes first', () => {
    const b = new SeriesBuffer()
    // Stream rows 3–5 arrive before the file (rows 0–3) has been read.
    b.append({ forces: [row(3), row(4)] })
    expect(b.table('forces')).toBeNull()
    b.append({ forces: [row(5)] })
    b.setFile('forces', FORCES + line(0) + line(1) + line(2) + line(3) + '4,0.0')
    const t = b.table('forces')!
    expect(t.data[0]).toEqual([0, 1, 2, 3, 4, 5])
    expect(t.data[4]).toEqual([0, 0.6, 1.2, 0.6 * 3, 0.6 * 4, 0.6 * 5])
    // Later rows are appended, repeats and rows of the wrong width ignored.
    b.append({ forces: [row(5), row(6), [7, 0.007]] })
    expect(t.data[0]).toEqual([0, 1, 2, 3, 4, 5, 6])
    expect(b.needsFile('forces')).toBe(false)
    expect(b.needsFile('probes')).toBe(true)
  })

  it('shows stream rows at once when hello named the columns', () => {
    const b = new SeriesBuffer()
    b.setColumns({ forces: FORCES.trim().split(','), probes: ['step', 'time_s'], diagnostics: [] })
    b.append({ forces: [row(10)] })
    expect(b.table('forces')?.data[0]).toEqual([10])
    // The file then fills in what came before.
    b.setFile('forces', FORCES + line(9) + line(10))
    expect(b.table('forces')?.data[0]).toEqual([9, 10])
  })

  it('reads the files again after a resync', () => {
    const b = new SeriesBuffer()
    b.setFile('forces', FORCES + line(0))
    b.forgetFiles()
    expect(b.needsFile('forces')).toBe(true)
    b.append({ forces: [row(8)] })
    b.setFile('forces', FORCES + line(0) + line(5))
    expect(b.table('forces')?.data[0]).toEqual([0, 5, 8])
  })
})

describe('plots', () => {
  const b = new SeriesBuffer()
  b.setFile('forces', FORCES + line(0) + line(10))
  b.setFile('probes', 'step,time_s,p1.ux_m_s,p1.uy_m_s,p1.p_Pa\n0,0,1,2,3\n')
  b.setFile('diagnostics', 'step,time_s,mass_lu,kinetic_energy_lu,max_velocity_m_s,max_mach\n')
  b.append({
    diagnostics: [
      [0, 0, 100, 0, 0, 0.1],
      [1, 0.1, 100.5, 0, 0, 0.12],
    ],
  })

  it('makes force coefficients from the reference scales', () => {
    expect(coefficientFactor(META)).toBeCloseTo(2 / (1.2 * 1 * 1 * 0.1), 12)
    // The domain edges are not an element to choose.
    expect(forceElements(b.table('forces')!)).toEqual(['cylinder'])
    const [p, ...rest] = forcePlots(b.table('forces'), META, 'cylinder')
    expect(rest).toEqual([])
    expect(p.title).toContain('cylinder')
    expect(p.time).toEqual([0, 0.01])
    expect(p.lines.map((l) => l.label)).toEqual(['C_D', 'C_L'])
    expect(p.lines[0].values[1]).toBeCloseTo((6 * 2) / 0.12, 12)
    expect(forcePlots(b.table('forces'), null, 'cylinder')).toEqual([])
    expect(forcePlots(b.table('forces'), META, 'nope')).toEqual([])
  })

  it('plots each probe quantity and the diagnostics', () => {
    expect(probePlots(b.table('probes')).map((p) => [p.title, p.lines[0].label])).toEqual([
      ['Probe u_x (m/s)', 'p1'],
      ['Probe u_y (m/s)', 'p1'],
      ['Probe pressure (Pa)', 'p1'],
    ])
    const [mass, mach] = diagnosticPlots(b.table('diagnostics'))
    expect(mass.lines[0].values).toEqual([0, 0.005])
    expect(mach.lines[0].values).toEqual([0.1, 0.12])
  })

  it('maps a time to the closest frame', () => {
    // Frames every 10 steps of 1 ms.
    expect(frameAt(0.024, META, 100)).toBe(2)
    expect(frameAt(0.026, META, 100)).toBe(3)
    expect(frameAt(-1, META, 100)).toBe(0)
    expect(frameAt(5, META, 7)).toBe(7)
  })
})

describe('elements', () => {
  it('finds the bodies: wall elements off the domain edge', () => {
    // 5 × 4 cells: element 1 (walls) along the bottom edge, 2 (a wall) inside, 3 (an inlet) inside.
    const ids = new Uint16Array(20)
    for (let x = 0; x < 5; x++) ids[x] = 1
    ids[1 * 5 + 2] = 2
    ids[2 * 5 + 2] = 3
    const wall = { kind: 'wall', thermal: { type: 'adiabatic' }, velocity: { type: 'noSlip' } }
    const elements = [
      { id: 1, name: 'Channel walls', ...wall },
      { id: 2, ...wall },
      { id: 3, name: 'jet', kind: 'inlet' },
    ] as unknown as Parameters<typeof bodies>[0]
    expect(bodies(elements, ids, 5, 4)).toEqual(['element2'])
    expect(columnName('Channel walls (top)')).toBe('Channel_walls__top_')
    expect(shownElement(['Channel_walls', 'element2'], ['element2'], null)).toBe('element2')
    expect(shownElement(['Channel_walls', 'element2'], ['element2'], 'Channel_walls')).toBe(
      'Channel_walls',
    )
    expect(shownElement(['a', 'b'], [], 'gone')).toBe('a')
    expect(shownElement([], [], null)).toBeNull()
  })
})

describe('SeriesFeed', () => {
  const job = { id: 'j', status: 'running' } as unknown as Extract<
    StreamMessage,
    { type: 'hello' }
  >['job']

  it('reads the files and meta once the job runs, and retries what failed', async () => {
    const asked: string[] = []
    let failMeta = true
    const feed = new SeriesFeed(
      {
        meta: async () => {
          asked.push('meta.json')
          if (failMeta) throw new Error('404')
          return META
        },
        text: async (name) => {
          asked.push(name)
          return name === 'forces.csv' ? FORCES + line(0) : 'step,time_s\n'
        },
      },
      () => {},
    )
    feed.onMessage({ type: 'hello', job: { ...job, status: 'queued' }, series: null })
    await Promise.resolve()
    expect(asked).toEqual([]) // queued: nothing to read yet
    feed.onMessage({ type: 'series', rows: { forces: [row(1)] } })
    await new Promise((r) => setTimeout(r, 0))
    expect(asked.sort()).toEqual(['diagnostics.csv', 'forces.csv', 'meta.json', 'probes.csv'])
    expect(feed.buffer.table('forces')?.data[0]).toEqual([0, 1])
    expect(feed.meta).toBeNull()

    failMeta = false
    asked.length = 0
    feed.onMessage({ type: 'series', rows: { forces: [row(2)] } })
    await new Promise((r) => setTimeout(r, 0))
    expect(asked).toEqual(['meta.json'])
    expect(feed.meta).toEqual(META)
    expect(feed.buffer.table('forces')?.data[0]).toEqual([0, 1, 2])
  })
})
