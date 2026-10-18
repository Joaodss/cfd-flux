// Number formatting shared by the panels.

export const CELL_NAMES: Record<number, string> = {
  0: 'Fluid',
  1: 'Solid',
  2: 'Inlet',
  3: 'Outlet',
  4: 'Heat source',
  5: 'Empty',
}

/** A length in metres with a sensible unit: `0.205 m`, `12.5 mm`, `3.2 µm`. */
export function formatLength(metres: number): string {
  const a = Math.abs(metres)
  if (a === 0) return '0 m'
  if (a >= 0.1) return `${trim(metres, 3)} m`
  if (a >= 1e-4) return `${trim(metres * 1e3, 3)} mm`
  return `${trim(metres * 1e6, 3)} µm`
}

function trim(v: number, digits: number): string {
  return Number(v.toPrecision(digits + 1)).toString()
}

/** Zoom readout: `8 px/cell`, `1.41 px/cell`; below 1 px/cell, `2.83 cells/px`. */
export function formatZoom(zoom: number): string {
  if (zoom >= 1) return `${Number(zoom.toFixed(2))} px/cell`
  return `${Number((1 / zoom).toFixed(2))} cells/px`
}

/** `512 B`, `3.4 MB`, `1.2 GB` (decimal units, as disks and GPUs are sold). */
export function formatBytes(bytes: number): string {
  const units = ['B', 'kB', 'MB', 'GB', 'TB']
  let v = bytes
  let u = 0
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000
    u++
  }
  return `${u === 0 ? v : Number(v.toPrecision(3))} ${units[u]}`
}

/** `45 s`, `3 min 20 s`, `2 h 05 min`. */
export function formatDuration(seconds: number): string {
  if (seconds < 1) return '< 1 s'
  const s = Math.round(seconds)
  if (s < 60) return `${s} s`
  if (s < 3600) return `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, '0')} s`
  return `${Math.floor(s / 3600)} h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')} min`
}

/** Three significant digits, or scientific notation for very large or small values. */
export function formatValue(v: number): string {
  const a = Math.abs(v)
  if (a !== 0 && (a >= 1e6 || a < 1e-3)) return v.toExponential(2)
  return String(Number(v.toPrecision(3)))
}
