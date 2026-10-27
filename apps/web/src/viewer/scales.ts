// The reference scales of the reported numbers (design proposal §7.4, Q1): the run's own, from
// `meta.json` (the scene's `reference` block, with the defaults filled in), or a per-session
// override typed in the Statistics tab — not saved in the scene or in the browser. Everything
// that uses the scales (C_D, C_L, St, Nu, the plots and the CSV exports) reads them from one
// modified copy of the meta, so they can never disagree.
import type { RunMeta } from '../api/jobs.ts'

/** Scales typed by the user; a missing one keeps the run's. */
export interface ScaleOverride {
  length?: number
  velocity?: number
  temperatureDifference?: number
}

export type ScaleKey = keyof ScaleOverride

/** Whether any scale is overridden. */
export function isOverridden(o: ScaleOverride): boolean {
  return o.length !== undefined || o.velocity !== undefined || o.temperatureDifference !== undefined
}

/**
 * `meta` with the overridden reference scales (the same object when nothing is overridden). The
 * Reynolds number follows (Re = U L / ν with the run's ν, so it scales with U L).
 */
export function withScales(meta: RunMeta, o: ScaleOverride): RunMeta {
  if (!isOverridden(o)) return meta
  const r = meta.dimensionless.reference
  const length = o.length ?? r.length
  const velocity = o.velocity ?? r.velocity
  return {
    ...meta,
    dimensionless: {
      ...meta.dimensionless,
      reference: {
        ...r,
        length,
        velocity,
        reynolds: (r.reynolds * (velocity * length)) / (r.velocity * r.length),
        temperatureDifference: o.temperatureDifference ?? r.temperatureDifference,
      },
    },
  }
}

/** Whether the run is thermal (it has a conductivity for the Nusselt number, or a `heat.csv`). */
export function isThermal(meta: RunMeta): boolean {
  return meta.run.heat === true || meta.dimensionless.reference.conductivity !== undefined
}
