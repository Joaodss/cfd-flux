// The view transform between screen pixels (CSS pixels, y down, origin at the canvas' top-left
// corner) and scene cells (y up, origin at the domain's bottom-left corner, 03 §3).

export interface View {
  /** CSS pixels per cell. */
  zoom: number
  /** Screen position of the domain's top-left corner (cell x = 0, y = height). */
  offsetX: number
  offsetY: number
}

/** Zoom limits: an overview of a 4096² domain up to 32 px per cell. */
export const MIN_ZOOM = 1 / 8
export const MAX_ZOOM = 32
/** Each wheel notch or `+`/`−` multiplies the zoom by √2. */
export const ZOOM_STEP = Math.SQRT2

/** The cell under a screen point (it may lie outside the grid). */
export function screenToCell(v: View, height: number, sx: number, sy: number): [number, number] {
  const u = (sx - v.offsetX) / v.zoom
  const w = (sy - v.offsetY) / v.zoom
  return [Math.floor(u), height - 1 - Math.floor(w)]
}

/** Screen position of a point given in (fractional) cell coordinates, y up. */
export function cellToScreen(v: View, height: number, x: number, y: number): [number, number] {
  return [v.offsetX + x * v.zoom, v.offsetY + (height - y) * v.zoom]
}

export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom))
}

/** Zooms to `zoom`, keeping the scene point under the screen point (sx, sy) where it is. */
export function zoomAt(v: View, zoom: number, sx: number, sy: number): View {
  const z = clampZoom(zoom)
  const k = z / v.zoom
  return { zoom: z, offsetX: sx - (sx - v.offsetX) * k, offsetY: sy - (sy - v.offsetY) * k }
}

/**
 * The next zoom level in `direction` (+1 in, −1 out) on the √2 ladder through 1 px/cell, so
 * repeated steps always land on 1, 2, 4, 8… exactly.
 */
export function stepZoom(zoom: number, direction: 1 | -1): number {
  const level = Math.log(zoom) / Math.log(ZOOM_STEP)
  const next = direction > 0 ? Math.floor(level + 1e-9) + 1 : Math.ceil(level - 1e-9) - 1
  // 2^(n/2) rather than √2^n: exact powers of two for even n.
  return clampZoom(2 ** (next / 2))
}

/**
 * The largest ladder zoom at which the grid fits in the viewport with a margin, centred.
 */
export function fitView(
  width: number,
  height: number,
  viewportWidth: number,
  viewportHeight: number,
  margin = 24,
): View {
  const avail = Math.min(
    (viewportWidth - 2 * margin) / width,
    (viewportHeight - 2 * margin) / height,
  )
  let zoom = MIN_ZOOM
  while (zoom < MAX_ZOOM && stepZoom(zoom, 1) <= avail) zoom = stepZoom(zoom, 1)
  return {
    zoom,
    offsetX: Math.round((viewportWidth - width * zoom) / 2),
    offsetY: Math.round((viewportHeight - height * zoom) / 2),
  }
}
