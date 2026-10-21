// The Statistics tab (design proposal §7.2): the force statistics of the chosen element over the
// window (the second half of the run so far, or what was dragged on a plot), compared with the
// benchmark the run's scene reproduces, with the tolerance the validation suite uses at the
// run's resolution.
import { useMemo, useState } from 'react'
import { Button } from '../components/ui/button.tsx'
import { formatValue } from '../panels/format.ts'
import { ElementSelect } from './PlotsPanel.tsx'
import { coefficients } from './plots.ts'
import {
  benchmarkOfScene,
  BENCHMARKS,
  check,
  isBenchmark,
  toleranceAt,
  type BenchmarkId,
} from './references.ts'
import { useElement, useSeries, useWindow } from './seriesFeed.ts'
import { forceStatistics, type ForceStatistics } from './statistics.ts'

const ROWS: { key: keyof ForceStatistics; label: string }[] = [
  { key: 'strouhal', label: 'St' },
  { key: 'cdMean', label: 'C_D mean' },
  { key: 'cdMax', label: 'C_D,max' },
  { key: 'clMean', label: 'C_L mean' },
  { key: 'clAmplitude', label: 'C_L amplitude' },
  { key: 'clMax', label: 'C_L,max' },
]

const SELECT = 'h-7 rounded-md border border-border bg-background px-1.5 text-xs'

function useStatistics(): ForceStatistics | null {
  const feed = useSeries((s) => s.feed)
  const version = useSeries((s) => s.version)
  const { element } = useElement()
  const { window } = useWindow()
  return useMemo(() => {
    void version
    const forces = feed?.buffer.table('forces')
    const meta = feed?.meta
    if (!forces || !meta || !element || !window) return null
    const c = coefficients(forces, element, meta)
    return c && forceStatistics(forces.data[1], c.cd, c.cl, window, meta)
  }, [feed, version, element, window])
}

const percent = (v: number) => `${v >= 0 ? '+' : ''}${(100 * v).toFixed(1)} %`

export function StatsPanel({ sceneName }: { sceneName: string | undefined }) {
  const stats = useStatistics()
  const meta = useSeries((s) => s.feed?.meta ?? null)
  const { window, chosen } = useWindow()
  // The benchmark to compare with: the example's by default, or the user's choice.
  const [picked, setPicked] = useState<BenchmarkId | 'none' | null>(null)
  const benchmarkId = picked ?? benchmarkOfScene(sceneName) ?? 'none'
  const benchmark = benchmarkId === 'none' ? null : BENCHMARKS[benchmarkId]
  // Resolution in cells per reference length, for the suite's tolerance.
  const cells = meta ? meta.dimensionless.reference.length / meta.units.dx : null
  const tolerance = benchmark && cells !== null ? toleranceAt(benchmark, cells) : null

  if (!meta || !window) {
    return <p className="text-sm text-muted-foreground">No force series yet.</p>
  }
  return (
    <div className="flex flex-col gap-3 text-sm" data-testid="stats">
      <ElementSelect />
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <span data-testid="stats-window">
          Window {formatValue(window.from)}–{formatValue(window.to)} s
          {chosen ? '' : ' (second half)'}
        </span>
        {chosen && (
          <Button variant="ghost" size="sm" onClick={() => useSeries.getState().setWindow(null)}>
            Reset
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">Drag across a plot to choose the window.</p>
      {stats ? (
        <table className="w-full text-xs tabular-nums">
          <thead className="text-muted-foreground">
            <tr className="text-left">
              <th className="font-normal">Quantity</th>
              <th className="font-normal">Value</th>
              {benchmark && <th className="font-normal">Reference</th>}
              {benchmark && <th className="font-normal">Error</th>}
            </tr>
          </thead>
          <tbody>
            {ROWS.map(({ key, label }) => {
              const value = stats[key] as number | null
              const c =
                benchmark && tolerance && value !== null
                  ? check(benchmark, key, value, tolerance.relative)
                  : null
              return (
                <tr key={key} data-testid={`stat-${key}`}>
                  <td className="py-0.5">{label}</td>
                  <td>{value === null ? '—' : value.toPrecision(4)}</td>
                  {benchmark && <td>{c ? `${c.interval[0]}–${c.interval[1]}` : ''}</td>}
                  {benchmark && (
                    <td data-testid={`stat-${key}-check`}>
                      {c && (
                        <span
                          className={
                            c.pass === true
                              ? 'text-green-500'
                              : c.pass === false
                                ? 'text-red-400'
                                : 'text-muted-foreground'
                          }
                        >
                          {percent(c.error)}
                          {c.pass === true ? ' ✓' : c.pass === false ? ' ✗' : ''}
                        </span>
                      )}
                    </td>
                  )}
                </tr>
              )
            })}
          </tbody>
        </table>
      ) : (
        <p className="text-xs text-muted-foreground">Too few samples in the window.</p>
      )}
      {stats && (
        <p className="text-xs text-muted-foreground">
          {stats.samples.toLocaleString('en')} samples, {stats.periods} periods
          {stats.frequency !== null && `, f = ${formatValue(stats.frequency)} Hz`}. Reference
          scales: L = {formatValue(meta.dimensionless.reference.length)} m, U ={' '}
          {formatValue(meta.dimensionless.reference.velocity)} m/s, ρ ={' '}
          {formatValue(meta.units.rho0)} kg/m³.
        </p>
      )}
      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        Compare with
        <select
          className={SELECT}
          data-testid="stats-benchmark"
          value={benchmarkId}
          onChange={(e) => {
            const v = e.target.value
            setPicked(isBenchmark(v) ? v : 'none')
          }}
        >
          <option value="none">nothing</option>
          {(Object.keys(BENCHMARKS) as BenchmarkId[]).map((id) => (
            <option key={id} value={id}>
              {BENCHMARKS[id].title}
            </option>
          ))}
        </select>
      </label>
      {benchmark && tolerance && cells !== null && (
        <p className="text-xs text-muted-foreground" data-testid="stats-tolerance">
          ✓ within {Math.round(100 * tolerance.relative)} % of the interval's centre: the validation
          suite's tolerance at {Math.round(cells)} cells per reference length
          {tolerance.coarser ? ' (coarser than any resolution the suite checks)' : ''}.{' '}
          {benchmark.citation}.
        </p>
      )}
    </div>
  )
}
