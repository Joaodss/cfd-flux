// Statistics of a force history over a window (design proposal §7.2): mean C_D, C_D,max, mean
// C_L, C_L amplitude, C_L,max and the Strouhal number. Post-processing of time series, so it runs
// here, where the window can be dragged; the frequency is `cfd_verify::harness::
// oscillation_frequency`'s algorithm, and a contract test runs both on the same `forces.csv`.
import type { RunMeta } from '../api/jobs.ts'
import type { Value } from './series.ts'

/** A time window in seconds, `from ≤ t ≤ to`. */
export interface Window {
  from: number
  to: number
}

/** The second half of the time so far (the default window). */
export function defaultWindow(time: Value[]): Window | null {
  const first = time[0]
  const last = time[time.length - 1]
  if (first === undefined || first === null || last === undefined || last === null) return null
  return { from: 0.5 * (first + last), to: last }
}

/**
 * Upward crossings of `y − mean(y)`, linearly interpolated in time (the same operations, in the
 * same order, as `cfd_verify::harness::upward_crossings`).
 */
export function upwardCrossings(t: number[], y: number[]): number[] {
  let sum = 0
  for (const v of y) sum += v
  const mean = sum / y.length
  const out: number[] = []
  for (let i = 1; i < y.length; i++) {
    if (y[i - 1] - mean < 0 && y[i] - mean >= 0) {
      const a = y[i - 1] - mean
      const b = y[i] - mean
      out.push(t[i - 1] + (t[i] - t[i - 1]) * (-a / (b - a)))
    }
  }
  return out
}

/** Frequency from the upward crossings: whole periods between the first and the last. */
export function oscillationFrequency(t: number[], y: number[]): number | null {
  const c = upwardCrossings(t, y)
  return c.length >= 2 ? (c.length - 1) / (c[c.length - 1] - c[0]) : null
}

export interface ForceStatistics {
  window: Window
  samples: number
  cdMean: number
  cdMax: number
  clMean: number
  /** Half the peak-to-peak of C_L. */
  clAmplitude: number
  clMax: number
  /** Shedding frequency (Hz) and St = f L / U; `null` with fewer than two crossings. */
  frequency: number | null
  strouhal: number | null
  /** Whole periods between the first and the last crossing. */
  periods: number
}

/** The statistics of C_D and C_L over the samples inside `window` (rows with gaps left out). */
export function forceStatistics(
  time: Value[],
  cd: Value[],
  cl: Value[],
  window: Window,
  meta: RunMeta,
): ForceStatistics | null {
  const t: number[] = []
  const d: number[] = []
  const l: number[] = []
  for (let i = 0; i < time.length; i++) {
    const ti = time[i]
    const di = cd[i]
    const li = cl[i]
    if (ti === null || di === null || li === null) continue
    if (ti < window.from || ti > window.to) continue
    t.push(ti)
    d.push(di)
    l.push(li)
  }
  if (t.length < 2) return null
  const mean = (v: number[]) => v.reduce((s, x) => s + x, 0) / v.length
  const max = (v: number[]) => v.reduce((m, x) => Math.max(m, x), -Infinity)
  const min = (v: number[]) => v.reduce((m, x) => Math.min(m, x), Infinity)
  const crossings = upwardCrossings(t, l)
  const frequency =
    crossings.length >= 2
      ? (crossings.length - 1) / (crossings[crossings.length - 1] - crossings[0])
      : null
  const { length, velocity } = meta.dimensionless.reference
  return {
    window,
    samples: t.length,
    cdMean: mean(d),
    cdMax: max(d),
    clMean: mean(l),
    clAmplitude: 0.5 * (max(l) - min(l)),
    clMax: max(l),
    frequency,
    strouhal: frequency === null ? null : (frequency * length) / velocity,
    periods: Math.max(0, crossings.length - 1),
  }
}
