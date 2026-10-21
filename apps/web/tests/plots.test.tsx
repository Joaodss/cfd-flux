// @vitest-environment happy-dom
// The Plots tab of the job view: the files and `meta.json` are read once the job runs, stream
// rows are appended, and each plot is a uPlot chart (replaced here by a stand-in that records its
// data: happy-dom has no 2D canvas).
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
  dimensionless: { reference: { length: 0.1, velocity: 1, reynolds: 100 } },
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
})
