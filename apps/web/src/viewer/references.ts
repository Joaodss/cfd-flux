// Benchmark reference values for the Statistics tab, generated from `cfd_verify::references`
// into `src/generated/references.json` by `cfd-cli references` (a Rust test keeps it current),
// with the relative tolerance the validation suite checks each one with at each resolution.
import references from '../generated/references.json' with { type: 'json' }
import { EXAMPLES } from '../scene/examples.ts'

export type BenchmarkId = keyof typeof references

export interface Benchmark {
  title: string
  citation: string
  /** Reference intervals `[lower, upper]`, by statistic (`strouhal`, `cdMax`, …). */
  quantities: Record<string, number[]>
  /** The statistics the suite passes or fails on; the others are for information. */
  checked: string[]
  /** Relative tolerance by resolution (cells per reference length), coarsest first. */
  tolerances: { cells: number; relative: number }[]
}

export const BENCHMARKS: Record<BenchmarkId, Benchmark> = references

export function isBenchmark(id: string | undefined): id is BenchmarkId {
  return id !== undefined && id in BENCHMARKS
}

/** The benchmark of a run whose scene has a gallery example's name, if that example has one. */
export function benchmarkOfScene(sceneName: string | undefined): BenchmarkId | null {
  const id = EXAMPLES.find((e) => e.scene.name === sceneName)?.benchmark
  return isBenchmark(id) ? id : null
}

/**
 * The tolerance at a resolution: the finest entry at or below it, else the coarsest (`coarser`:
 * the run is coarser than anything the suite checks).
 */
export function toleranceAt(b: Benchmark, cells: number): { relative: number; coarser: boolean } {
  const fits = b.tolerances.filter((t) => t.cells <= cells + 1e-9)
  const t = fits[fits.length - 1] ?? b.tolerances[0]
  return { relative: t.relative, coarser: fits.length === 0 }
}

export interface Check {
  interval: [number, number]
  /** Relative to the interval's centre, as the suite reports it. */
  error: number
  /** `null` for a statistic shown for information only. */
  pass: boolean | null
}

/** A statistic against its reference: the suite's rule, |value / centre − 1| < tolerance. */
export function check(
  b: Benchmark,
  quantity: string,
  value: number,
  relative: number,
): Check | null {
  const q = b.quantities[quantity]
  if (!q) return null
  const interval: [number, number] = [q[0], q[1]]
  const centre = 0.5 * (interval[0] + interval[1])
  const error = value / centre - 1
  return {
    interval,
    error,
    pass: b.checked.includes(quantity) ? Math.abs(error) < relative : null,
  }
}
