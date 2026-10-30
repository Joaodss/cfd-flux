// @vitest-environment happy-dom
// The Plots tab of the job view: the files and `meta.json` are read once the job runs, stream
// rows are appended, and each plot is a uPlot chart (replaced here by a stand-in that records its
// data: happy-dom has no 2D canvas).
import { readFileSync } from 'node:fs'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Job, RunMeta } from '../src/api/jobs.ts'
import { JobView } from '../src/app/JobView.tsx'

const charts = vi.hoisted(() => [] as { title: string; data: unknown[][] }[])
vi.mock('uplot', () => ({
  default: class {
    over = document.createElement('div')
    cursor = { left: -1 }
    record: { title: string; data: unknown[][] }
    constructor(opts: { title: string }, data: unknown[][], el: HTMLElement) {
      this.record = { title: opts.title, data }
      charts.push(this.record)
      el.appendChild(this.over)
    }
    setData(data: unknown[][]) {
      this.record.data = data
    }
    setSize() {}
    redraw() {}
    destroy() {}
  },
}))

const job: Job = {
  id: 'job-1',
  status: 'running',
  scene: 'Cylinder Re=100',
  backend: 'cpu',
  stepsTotal: 100,
  stepsDone: 0,
  frames: 0,
  createdAt: 0,
}

const META: RunMeta = {
  units: { dx: 0.005, dt: 0.001, rho0: 1.2 },
  dimensionless: {
    reference: { length: 0.1, velocity: 1, reynolds: 100, temperatureDifference: 1 },
  },
  run: { outputEverySteps: 10, sampleEverySteps: 1, endTime: 0.1 },
}

const FORCE_COLUMNS = ['step', 'time_s', 'cylinder.fx_N_m', 'cylinder.fy_N_m']

class FakeSocket {
  static last: FakeSocket
  binaryType = 'blob'
  onmessage: ((e: { data: unknown }) => void) | null = null
  onclose: (() => void) | null = null
  constructor() {
    FakeSocket.last = this
  }
  send(m: object) {
    act(() => this.onmessage?.({ data: JSON.stringify(m) }))
  }
  close() {}
}

const FILES: Record<string, string> = {
  'meta.json': JSON.stringify(META),
  'forces.csv': `${FORCE_COLUMNS.join(',')}\n0,0,0.06,0\n1,0.001,0.12,0.006\n`,
  'probes.csv': 'step,time_s\n',
  'diagnostics.csv': 'step,time_s,mass_lu,kinetic_energy_lu,max_velocity_m_s,max_mach\n',
  'scene.json': '{}',
}

describe('Plots tab', () => {
  beforeEach(() => {
    charts.length = 0
    vi.stubGlobal('WebSocket', FakeSocket)
    vi.stubGlobal(
      'ResizeObserver',
      class {
        observe() {}
        disconnect() {}
      },
    )
    vi.stubGlobal('fetch', async (url: string) => {
      const name = url.split('/').pop()!
      return name in FILES
        ? new Response(FILES[name], { status: 200 })
        : new Response(null, { status: 404 })
    })
  })
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('plots the force coefficients from the files and the stream', async () => {
    render(<JobView id="job-1" />)
    FakeSocket.last.send({
      type: 'hello',
      job,
      series: { forces: FORCE_COLUMNS, probes: ['step', 'time_s'], diagnostics: [] },
    })
    // The files are read in the background; let them land.
    await act(async () => new Promise((r) => setTimeout(r, 10)))
    FakeSocket.last.send({ type: 'series', rows: { forces: [[2, 0.002, 0.18, -0.006]] } })
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-plots'))
      fireEvent.click(screen.getByTestId('job-tab-plots'))
    })
    expect(screen.getByTestId('plot-forces:cylinder')).toBeTruthy()
    const forces = charts.find((c) => c.title.includes('cylinder'))!
    const [time, cd, cl] = forces.data as number[][]
    expect(time).toEqual([0, 0.001, 0.002])
    // C = 2F / (ρ U² L) = F / 0.06.
    expect(cd.map((v) => +v.toFixed(9))).toEqual([1, 2, 3])
    expect(cl.map((v) => +v.toFixed(9))).toEqual([0, 0.1, -0.1])
  })

  it('shows the statistics of the second half against the benchmark of the example', async () => {
    // A finished run of the cylinder example: 8.5–10 s of its real forces, D = 20 cells.
    FILES['forces.csv'] = readFileSync('tests/fixtures/series/cylinder-forces.csv', 'utf8') // from apps/web
    FILES['meta.json'] = JSON.stringify({ ...META, units: { dx: 0.005, dt: 1e-4, rho0: 1 } })
    render(<JobView id="job-1" />)
    FakeSocket.last.send({ type: 'hello', job: { ...job, status: 'completed' } })
    FakeSocket.last.send({ type: 'done', status: 'completed', stepsDone: 100, frames: 10 })
    await act(async () => new Promise((r) => setTimeout(r, 10)))
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-stats'))
      fireEvent.click(screen.getByTestId('job-tab-stats'))
    })
    expect(screen.getByTestId('stats-window').textContent).toBe('Window 9.25–10 s (second half)')
    expect(screen.getByTestId('stats-benchmark')).toHaveProperty('value', 'schaferTurek2D2')
    expect(screen.getByTestId('stat-strouhal').textContent).toMatch(/^St0\.297/)
    expect(screen.getByTestId('stat-strouhal-check').textContent).toMatch(/^−?-?0\.\d % ✓$/)
    expect(screen.getByTestId('stat-cdMax-check').textContent).toMatch(/^\+4\.\d % ✓$/)
    expect(screen.getByTestId('stat-clMax-check').textContent).not.toMatch(/[✓✗]/)
    expect(screen.getByTestId('stats-tolerance').textContent).toMatch(/5 %.* 20 cells/)
  })

  it('recomputes the statistics from reference scales overridden for the session', async () => {
    FILES['forces.csv'] = readFileSync('tests/fixtures/series/cylinder-forces.csv', 'utf8')
    FILES['meta.json'] = JSON.stringify({ ...META, units: { dx: 0.005, dt: 1e-4, rho0: 1 } })
    render(<JobView id="job-1" />)
    FakeSocket.last.send({ type: 'hello', job: { ...job, status: 'completed' } })
    FakeSocket.last.send({ type: 'done', status: 'completed', stepsDone: 100, frames: 10 })
    await act(async () => new Promise((r) => setTimeout(r, 10)))
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-stats'))
      fireEvent.click(screen.getByTestId('job-tab-stats'))
    })
    const value = (key: string) =>
      Number(screen.getByTestId(`stat-${key}`).querySelectorAll('td')[1].textContent)
    const [cd, st] = [value('cdMax'), value('strouhal')]
    const u = screen.getByTestId('stats-scale-velocity') as HTMLInputElement
    expect(u.value).toBe('1')
    expect(screen.queryByTestId('stats-scales-overridden')).toBeNull()
    // Twice the velocity: C_D / 4, St / 2; the tolerance now names the length in use.
    act(() => {
      fireEvent.focus(u)
      fireEvent.change(u, { target: { value: '2' } })
      fireEvent.keyDown(u, { key: 'Enter' })
    })
    expect(screen.getByTestId('stats-scales-overridden')).toBeTruthy()
    expect(value('cdMax') / cd).toBeCloseTo(0.25, 3)
    expect(value('strouhal') / st).toBeCloseTo(0.5, 3)
    expect(screen.getByTestId('stat-cdMax-check').textContent).toMatch(/✗$/)
    const l = screen.getByTestId('stats-scale-length') as HTMLInputElement
    act(() => {
      fireEvent.focus(l)
      fireEvent.change(l, { target: { value: '0.05' } })
      fireEvent.keyDown(l, { key: 'Enter' })
    })
    expect(screen.getByTestId('stats-tolerance').textContent).toMatch(/ 10 cells .*overridden/)
    // Reset: the run's scales and numbers again.
    act(() => fireEvent.click(screen.getByTestId('stats-scales-reset')))
    expect(screen.queryByTestId('stats-scales-overridden')).toBeNull()
    expect(value('cdMax')).toBe(cd)
    expect(screen.getByTestId('stats-tolerance').textContent).toMatch(
      / 20 cells per reference length\./,
    )
  })

  it('shows the Nusselt numbers of a thermal run against de Vahl Davis', async () => {
    // The quick heated cavity: 40–80 s of its real heat.csv, L = 0.1 m = 64 cells.
    FILES['heat.csv'] = readFileSync('tests/fixtures/series/cavity-heat.csv', 'utf8')
    FILES['forces.csv'] =
      'step,time_s,Hot_wall.fx_N_m,Hot_wall.fy_N_m,Cold_wall.fx_N_m,Cold_wall.fy_N_m\n0,0,0,0,0,0\n1,80,0,0,0,0\n'
    FILES['meta.json'] = JSON.stringify({
      units: { dx: 0.1 / 64, dt: 0.001, rho0: 1.2 },
      dimensionless: {
        reference: {
          length: 0.1,
          velocity: 0.1,
          reynolds: 374,
          temperatureDifference: 0.9572000000000003,
          conductivity: 0.0257,
        },
      },
      run: { outputEverySteps: 10, sampleEverySteps: 1, endTime: 80, heat: true },
    })
    render(<JobView id="job-1" />)
    FakeSocket.last.send({
      type: 'hello',
      job: { ...job, status: 'completed', scene: 'Heated cavity Ra=1e5 (quick)' },
    })
    FakeSocket.last.send({ type: 'done', status: 'completed', stepsDone: 100, frames: 10 })
    await act(async () => new Promise((r) => setTimeout(r, 10)))
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-stats'))
      fireEvent.click(screen.getByTestId('job-tab-stats'))
    })
    expect(screen.getByTestId('stats-benchmark')).toHaveProperty('value', 'deVahlDavis1e5')
    // No cylinder statistics for a run without bodies.
    expect(screen.queryByTestId('stat-strouhal')).toBeNull()
    expect(screen.getByTestId('nu-Hot_wall').textContent).toMatch(/^Hot wall4.54/)
    expect(screen.getByTestId('nu-Hot_wall-check').textContent).toBe('+0.7 % ✓')
    // The cold wall takes the heat the hot one gives (Nu < 0) and has no reference of its own.
    expect(screen.getByTestId('nu-Cold_wall').textContent).toMatch(/^Cold wall-4.54/)
    expect(screen.getByTestId('nu-Cold_wall-check').textContent).toBe('')
    expect(screen.getByTestId('stats-tolerance').textContent).toMatch(/2 %.* 64 cells/)

    // Nu(t) of both walls in the Plots tab, the hot one settled near the window's 4.54.
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-plots'))
      fireEvent.click(screen.getByTestId('job-tab-plots'))
    })
    expect(screen.getByTestId('plot-heat:nusselt')).toBeTruthy()
    const nu = charts.find((c) => c.title === 'Nusselt number')!
    const [time, hot, cold] = nu.data as number[][]
    expect(time.length).toBe(40)
    expect(hot.every((v) => Math.abs(v - 4.54) < 0.04)).toBe(true)
    expect(cold.every((v) => Math.abs(v + 4.54) < 0.04)).toBe(true)

    // heat.csv is one of the run's files, and Nu(t) has a CSV of its own.
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-export'))
      fireEvent.click(screen.getByTestId('job-tab-export'))
    })
    expect(screen.getByText('heat.csv').getAttribute('href')).toBe(
      '/api/simulations/job-1/files/heat.csv',
    )
    expect(screen.getByTestId('export-nu')).toBeTruthy()
  })

  it('offers no heat.csv and no Nu for a run without thermal walls', async () => {
    FILES['forces.csv'] = readFileSync('tests/fixtures/series/cylinder-forces.csv', 'utf8')
    FILES['meta.json'] = JSON.stringify(META)
    render(<JobView id="job-1" />)
    FakeSocket.last.send({ type: 'hello', job: { ...job, status: 'completed' } })
    FakeSocket.last.send({ type: 'done', status: 'completed', stepsDone: 100, frames: 10 })
    await act(async () => new Promise((r) => setTimeout(r, 10)))
    await act(async () => {
      fireEvent.mouseDown(screen.getByTestId('job-tab-export'))
      fireEvent.click(screen.getByTestId('job-tab-export'))
    })
    expect(screen.getByText('forces.csv')).toBeTruthy()
    expect(screen.queryByText('heat.csv')).toBeNull()
    expect(screen.queryByTestId('export-nu')).toBeNull()
    expect(charts.find((c) => c.title === 'Nusselt number')).toBeUndefined()
  })
})
