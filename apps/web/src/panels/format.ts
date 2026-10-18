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
