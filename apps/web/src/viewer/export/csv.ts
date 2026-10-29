// CSV exports (design proposal §8): the force coefficients, the window statistics, the walls'
// Nusselt numbers and the fields of the frame shown. Pure functions returning text; the run's own
// files are plain downloads.
import type { RunMeta } from '../../api/jobs.ts'
import { halfToFloat } from '../../frames/half.ts'
import type { DecodedFrame } from '../../frames/lffr.ts'
import { coefficients } from '../plots.ts'
import type { Table } from '../series.ts'
import { nusseltSeries, type ForceStatistics, type NusseltScales } from '../statistics.ts'

/** A number as short CSV text: 7 significant digits, `NaN` for gaps. */
export function num(v: number | null | undefined): string {
  return v === null || v === undefined || !Number.isFinite(v)
    ? 'NaN'
    : String(Number(v.toPrecision(7)))
}

/** Step, time and the coefficients of one element: `step,time_s,cd,cl`. Null without that element. */
export function coefficientsCsv(forces: Table, element: string, meta: RunMeta): string | null {
  const c = coefficients(forces, element, meta)
  if (!c) return null
  const lines = ['step,time_s,cd,cl']
  const [step, time] = forces.data
  for (let i = 0; i < time.length; i++) {
    lines.push([step[i], time[i], c.cd[i], c.cl[i]].map(num).join(','))
  }
  return lines.join('\n') + '\n'
}

const STATISTIC_ROWS: [string, (s: ForceStatistics) => number | null][] = [
  ['samples', (s) => s.samples],
  ['strouhal', (s) => s.strouhal],
  ['frequency_hz', (s) => s.frequency],
  ['periods', (s) => s.periods],
  ['cd_mean', (s) => s.cdMean],
  ['cd_max', (s) => s.cdMax],
  ['cl_mean', (s) => s.clMean],
  ['cl_amplitude', (s) => s.clAmplitude],
  ['cl_max', (s) => s.clMax],
]

/** The statistics of a window as `name,value` lines, with the element and the window first. */
export function statisticsCsv(stats: ForceStatistics, element: string): string {
  const lines = [
    'name,value',
    `element,${element}`,
    `window_from_s,${num(stats.window.from)}`,
    `window_to_s,${num(stats.window.to)}`,
    ...STATISTIC_ROWS.map(([name, f]) => `${name},${num(f(stats))}`),
  ]
  return lines.join('\n') + '\n'
}

/**
 * The fields of a frame, one row per block (`k × k` cells): the block centre in metres, then each
 * field's values (a vector as `name_x,name_y`). Values are means over the block's fluid cells
 * (the fluid fraction undoes the box filter's dilution); blocks without fluid are left empty.
 */
export function frameCsv(
  frame: DecodedFrame,
  fraction: Float32Array | null,
  grid: { width: number; height: number; cellSize: number },
  k: number,
): string {
  const { width: bw, height: bh } = frame.header
  const columns = ['x_m', 'y_m']
  for (const f of frame.fields) {
    if (f.components === 1) columns.push(f.name)
    else for (let c = 0; c < f.components; c++) columns.push(`${f.name}_${'xyz'[c] ?? c}`)
  }
  const lines = [columns.join(',')]
  for (let j = 0; j < bh; j++) {
    for (let i = 0; i < bw; i++) {
      const b = j * bw + i
      const frac = fraction ? fraction[b] : 1
      // Centre of the block's cells (the last blocks may be cut by the domain).
      const x = (Math.min((i + 1) * k, grid.width) + i * k) / 2
      const y = (Math.min((j + 1) * k, grid.height) + j * k) / 2
      const row = [num(x * grid.cellSize), num(y * grid.cellSize)]
      for (const f of frame.fields) {
        for (let c = 0; c < f.components; c++) {
          if (!(frac > 0)) {
            row.push('')
            continue
          }
          const raw = f.data[b * f.components + c]
          row.push(num((f.dtype === 'f16' ? halfToFloat(raw) : raw) / frac))
        }
      }
      lines.push(row.join(','))
    }
  }
  return lines.join('\n') + '\n'
}

/**
 * Step, time and the Nusselt number of every thermal wall: `step,time_s,{element}.nu,…`. Null
 * without thermal walls.
 */
export function nusseltCsv(heat: Table, scales: NusseltScales): string | null {
  const series = nusseltSeries(heat.columns, heat.data, scales)
  if (series.length === 0) return null
  const lines = [['step', 'time_s', ...series.map((s) => `${s.element}.nu`)].join(',')]
  const [step, time] = heat.data
  for (let i = 0; i < time.length; i++) {
    lines.push([step[i], time[i], ...series.map((s) => s.values[i])].map(num).join(','))
  }
  return lines.join('\n') + '\n'
}

/** Files of the run the server stores, offered as they are (`heat.csv` when the run has one). */
export function runFiles(meta: RunMeta | null): string[] {
  return [
    'forces.csv',
    'probes.csv',
    'diagnostics.csv',
    ...(meta?.run.heat ? ['heat.csv'] : []),
    'meta.json',
    'scene.json',
  ]
}
