// What the Plots tab draws, derived from the run's series (design proposal §7.1): the force
// coefficients of every element, the probes, the mass drift and the largest Mach number.
//
// Force coefficients use the scene's reference scales (`meta.json`'s `dimensionless.reference`,
// with the defaults filled in): C = 2F / (ρ U² L), forces per unit span (N/m).
import type { RunMeta } from '../api/jobs.ts'
import type { Table, Value } from './series.ts'

export interface PlotLine {
  label: string
  values: Value[]
}

export interface Plot {
  /** Stable across updates (the chart is reused while the run goes on). */
  id: string
  title: string
  /** Simulated time (s), the x of every line. */
  time: Value[]
  lines: PlotLine[]
}

/** 2 / (ρ U² L): newtons per metre of span → force coefficient. */
export function coefficientFactor(meta: RunMeta): number {
  const { length, velocity } = meta.dimensionless.reference
  return 2 / (meta.units.rho0 * velocity * velocity * length)
}

/** The elements of `forces.csv` that can carry a force (the domain edges are left out). */
export function forceElements(forces: Table): string[] {
  return forces.columns
    .filter((c) => c.endsWith('.fx_N_m'))
    .map((c) => c.slice(0, -'.fx_N_m'.length))
    .filter((n) => n !== 'edges')
}

/** C_D and C_L of an element over time (x and y of its force over the reference scales). */
export function coefficients(
  forces: Table,
  element: string,
  meta: RunMeta,
): { cd: Value[]; cl: Value[] } | null {
  const ix = forces.columns.indexOf(`${element}.fx_N_m`)
  const iy = forces.columns.indexOf(`${element}.fy_N_m`)
  if (ix < 0 || iy < 0) return null
  const k = coefficientFactor(meta)
  const scale = (v: Value) => (v === null ? null : k * v)
  return { cd: forces.data[ix].map(scale), cl: forces.data[iy].map(scale) }
}

/** C_D and C_L of one element (see `elements.ts` for which). */
export function forcePlots(
  forces: Table | null,
  meta: RunMeta | null,
  element: string | null,
): Plot[] {
  const c = forces && meta && element ? coefficients(forces, element, meta) : null
  if (!forces || !c) return []
  return [
    {
      id: `forces:${element}`,
      title: `C_D, C_L — ${element}`,
      time: forces.data[1],
      lines: [
        { label: 'C_D', values: c.cd },
        { label: 'C_L', values: c.cl },
      ],
    },
  ]
}

const PROBE_QUANTITIES = [
  ['ux_m_s', 'Probe u_x (m/s)', 'u_x'],
  ['uy_m_s', 'Probe u_y (m/s)', 'u_y'],
  ['p_Pa', 'Probe pressure (Pa)', 'p'],
  ['T_K', 'Probe temperature (K)', 'T'],
] as const

/** One plot per probe quantity, a line per probe. */
export function probePlots(probes: Table | null): Plot[] {
  if (!probes) return []
  const out: Plot[] = []
  for (const [suffix, title] of PROBE_QUANTITIES) {
    const lines = probes.columns.flatMap((c, i) =>
      c.endsWith(`.${suffix}`)
        ? [{ label: c.slice(0, -suffix.length - 1), values: probes.data[i] }]
        : [],
    )
    if (lines.length > 0) out.push({ id: `probes:${suffix}`, title, time: probes.data[1], lines })
  }
  return out
}

/** Mass drift ((m − m₀) / m₀, from the first row held) and the largest lattice Mach number. */
export function diagnosticPlots(diagnostics: Table | null): Plot[] {
  if (!diagnostics) return []
  const out: Plot[] = []
  const mass = diagnostics.data[diagnostics.columns.indexOf('mass_lu')]
  const m0 = mass?.[0]
  if (mass && m0) {
    out.push({
      id: 'diagnostics:mass',
      title: 'Mass drift',
      time: diagnostics.data[1],
      lines: [{ label: 'drift', values: mass.map((m) => (m === null ? null : (m - m0) / m0)) }],
    })
  }
  const mach = diagnostics.data[diagnostics.columns.indexOf('max_mach')]
  if (mach) {
    out.push({
      id: 'diagnostics:mach',
      title: 'Max Mach (lattice)',
      time: diagnostics.data[1],
      lines: [{ label: 'Ma', values: mach }],
    })
  }
  return out
}

/** The frame closest to simulated time `t` (frames every `outputEverySteps` steps from step 0). */
export function frameAt(t: number, meta: RunMeta, last: number): number {
  const interval = meta.run.outputEverySteps * meta.units.dt
  const i = Math.round(t / interval)
  return Math.max(0, Math.min(last, i))
}
