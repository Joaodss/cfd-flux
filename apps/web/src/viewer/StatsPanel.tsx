// The Statistics tab (design proposal §7.2): the force statistics of the chosen element over the
// window (the second half of the run so far, or what was dragged on a plot), compared with the
// benchmark the run's scene reproduces, with the tolerance the validation suite uses at the
// run's resolution. The reference scales are the run's, or overridden for this session (not
// saved): every coefficient, St and Nu, the plots and the CSV exports follow them.
import { useState } from 'react'
import { NumberInput } from '../components/fields.tsx'
import { Button } from '../components/ui/button.tsx'
import { formatValue } from '../panels/format.ts'
import { ElementSelect } from './PlotsPanel.tsx'
import {
  benchmarkOfScene,
  BENCHMARKS,
  check,
  type Benchmark,
  isBenchmark,
  toleranceAt,
  type BenchmarkId,
} from './references.ts'
import { isOverridden, isThermal, type ScaleKey } from './scales.ts'
import { useMeta, useNusselt, useSeries, useStatistics, useWindow } from './seriesFeed.ts'
import type { ForceStatistics } from './statistics.ts'

const ROWS: { key: keyof ForceStatistics; label: string }[] = [
  { key: 'strouhal', label: 'St' },
  { key: 'cdMean', label: 'C_D mean' },
  { key: 'cdMax', label: 'C_D,max' },
  { key: 'clMean', label: 'C_L mean' },
  { key: 'clAmplitude', label: 'C_L amplitude' },
  { key: 'clMax', label: 'C_L,max' },
]

const SELECT = 'h-7 rounded-md border border-border bg-background px-1.5 text-xs'

const percent = (v: number) => `${v >= 0 ? '+' : ''}${(100 * v).toFixed(1)} %`

/**
 * The Nusselt numbers of the thermal walls (design proposal §7.3): `Nu = q L / (k ΔT · length)`
 * from `heat.csv`, averaged over the window. The heated cavity is steady, so the mean is the
 * value; the spread says how steady the window is. The heated element (the one that gives heat
 * to the fluid) is compared with the benchmark's Nusselt number on the hot wall.
 */
function NusseltTable({
  benchmark,
  relative,
}: {
  benchmark: Benchmark | null
  relative: number | null
}) {
  const walls = useNusselt()
  if (walls.length === 0) return null
  const heated = walls.reduce((a, b) => (b.mean > a.mean ? b : a))
  const quantity = benchmark && 'nuHot' in benchmark.quantities ? 'nuHot' : null
  return (
    <div className="flex flex-col gap-1" data-testid="nusselt">
      <table className="w-full text-xs tabular-nums">
        <thead className="text-muted-foreground">
          <tr className="text-left">
            <th className="font-normal">Wall</th>
            <th className="font-normal">Nu (mean)</th>
            <th className="font-normal">Spread</th>
            {quantity && <th className="font-normal">Reference</th>}
            {quantity && <th className="font-normal">Error</th>}
          </tr>
        </thead>
        <tbody>
          {walls.map((w) => {
            const c =
              quantity && benchmark && relative !== null && w === heated
                ? check(benchmark, quantity, w.mean, relative)
                : null
            return (
              <tr key={w.element} data-testid={`nu-${w.element}`}>
                <td className="py-0.5">{w.element.replace(/_/g, ' ')}</td>
                <td>{w.mean.toPrecision(4)}</td>
                <td>{(100 * w.spread).toPrecision(2)} %</td>
                {quantity && <td>{c ? c.interval[0] : ''}</td>}
                {quantity && (
                  <td data-testid={`nu-${w.element}-check`}>
                    {c && (
                      <span className={c.pass === false ? 'text-red-400' : 'text-green-500'}>
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
      <p className="text-xs text-muted-foreground">
        Nu = q L / (k ΔT · length) per wall, mean over {walls[0].samples} frames of the window (a
        steady flow has settled: the spread is (max − min) / mean). Negative: the wall takes heat
        from the fluid.
      </p>
    </div>
  )
}

const SCALES: { key: ScaleKey; label: string; unit: string; thermal?: boolean }[] = [
  { key: 'length', label: 'Length L', unit: 'm' },
  { key: 'velocity', label: 'Velocity U', unit: 'm/s' },
  { key: 'temperatureDifference', label: 'ΔT', unit: 'K', thermal: true },
]

/**
 * The reference scales in use, editable for this session: pre-filled with the run's (from
 * `meta.json`), marked when overridden, and reset to the run's with one click.
 */
function ScalesEditor() {
  const run = useSeries((s) => s.feed?.meta ?? null)
  const scales = useSeries((s) => s.scales)
  const setScales = useSeries((s) => s.setScales)
  if (!run) return null
  const reference = run.dimensionless.reference
  const thermal = isThermal(run)
  const overridden = isOverridden(scales)
  return (
    <fieldset className="flex flex-col gap-1 text-xs" data-testid="stats-scales">
      <legend className="mb-1 flex w-full items-center gap-2 text-muted-foreground">
        Reference scales
        {overridden && (
          <span
            className="rounded-full bg-amber-500/15 px-2 py-0.5 text-amber-500"
            data-testid="stats-scales-overridden"
            title="Changed for this session only: the scene and the run keep theirs"
          >
            overridden
          </span>
        )}
        {overridden && (
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto"
            data-testid="stats-scales-reset"
            onClick={() => setScales({})}
          >
            Reset
          </Button>
        )}
      </legend>
      {SCALES.filter((f) => thermal || !f.thermal).map((f) => (
        <label key={f.key} className="grid grid-cols-[6rem_1fr_2.5rem] items-center gap-2">
          <span className="text-muted-foreground">{f.label}</span>
          <NumberInput
            label={f.label}
            testId={`stats-scale-${f.key}`}
            rules={{ positive: true }}
            value={scales[f.key] ?? reference[f.key]}
            onCommit={(v) => {
              // Back to the run's value: no longer overridden.
              const { [f.key]: _, ...rest } = scales
              setScales(v === reference[f.key] ? rest : { ...scales, [f.key]: v })
            }}
          />
          <span className="text-muted-foreground">{f.unit}</span>
        </label>
      ))}
    </fieldset>
  )
}

export function StatsPanel({ sceneName }: { sceneName: string | undefined }) {
  const stats = useStatistics()
  const nuWalls = useNusselt()
  const bodies = useSeries((s) => s.bodies)
  // A thermal run without bodies (the heated cavity) has no force coefficients to show.
  const showForces = bodies.length > 0 || nuWalls.length === 0
  const meta = useMeta()
  const overridden = isOverridden(useSeries((s) => s.scales))
  const { window, chosen } = useWindow()
  // The benchmark to compare with: the example's by default, or the user's choice.
  const [picked, setPicked] = useState<BenchmarkId | 'none' | null>(null)
  const benchmarkId = picked ?? benchmarkOfScene(sceneName) ?? 'none'
  const benchmark = benchmarkId === 'none' ? null : BENCHMARKS[benchmarkId]
  // Resolution in cells per reference length (the one in use), for the suite's tolerance.
  const cells = meta ? meta.dimensionless.reference.length / meta.units.dx : null
  const tolerance = benchmark && cells !== null ? toleranceAt(benchmark, cells) : null

  if (!meta || !window) {
    return <p className="text-sm text-muted-foreground">No force series yet.</p>
  }
  return (
    <div className="flex flex-col gap-3 text-sm" data-testid="stats">
      {showForces && <ElementSelect />}
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
      <ScalesEditor />
      <NusseltTable benchmark={benchmark} relative={tolerance?.relative ?? null} />
      {!showForces ? null : stats ? (
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
      {showForces && stats && (
        <p className="text-xs text-muted-foreground">
          {stats.samples.toLocaleString('en')} samples, {stats.periods} periods
          {stats.frequency !== null && `, f = ${formatValue(stats.frequency)} Hz`}.{' '}
          {overridden ? 'Overridden reference scales' : "The run's reference scales"}: L ={' '}
          {formatValue(meta.dimensionless.reference.length)} m, U ={' '}
          {formatValue(meta.dimensionless.reference.velocity)} m/s, ρ ={' '}
          {formatValue(meta.units.rho0)} kg/m³ (Re ={' '}
          {formatValue(meta.dimensionless.reference.reynolds)}).
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
          {overridden ? ' (the length in use, overridden)' : ''}
          {tolerance.coarser ? ' (coarser than any resolution the suite checks)' : ''}.{' '}
          {benchmark.citation}.
        </p>
      )}
    </div>
  )
}
