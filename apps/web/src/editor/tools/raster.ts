// Pixel-exact rasterisation of the drawing tools' shapes, in cell coordinates. Every function
// reports cells through a callback, so the same code draws into the layers and runs in tests.

/** Receives a horizontal run of cells `x0..x1` (inclusive) on row `y`. */
export type SpanFn = (y: number, x0: number, x1: number) => void
export type PlotFn = (x: number, y: number) => void

/** Bresenham's line from (x0, y0) to (x1, y1), both ends included, 8-connected. */
export function line(x0: number, y0: number, x1: number, y1: number, plot: PlotFn): void {
  const dx = Math.abs(x1 - x0)
  const dy = -Math.abs(y1 - y0)
  const sx = x0 < x1 ? 1 : -1
  const sy = y0 < y1 ? 1 : -1
  let err = dx + dy
  for (;;) {
    plot(x0, y0)
    if (x0 === x1 && y0 === y1) return
    const e2 = 2 * err
    if (e2 >= dy) {
      err += dy
      x0 += sx
    }
    if (e2 <= dx) {
      err += dx
      y0 += sy
    }
  }
}

/** Snaps the end of a line from (x0, y0) to the nearest 0°, 45° or 90° direction (Shift). */
export function snapAngle(x0: number, y0: number, x1: number, y1: number): [number, number] {
  const dx = x1 - x0
  const dy = y1 - y0
  const ax = Math.abs(dx)
  const ay = Math.abs(dy)
  if (ay * 2.414 < ax) return [x1, y0] // within 22.5° of horizontal (tan 67.5° = 2.414)
  if (ax * 2.414 < ay) return [x0, y1]
  const d = Math.max(ax, ay)
  return [x0 + Math.sign(dx) * d, y0 + Math.sign(dy) * d]
}

/** A brush: a square or round footprint of `size` cells across, centred on the pointer's cell. */
export interface Brush {
  size: number
  round: boolean
}

/**
 * The brush's rows as offsets from the centre cell: `[dy, dx0, dx1]`. Even sizes extend one more
 * cell towards +x/+y. A round brush keeps the cells whose centres lie within the brush's circle.
 */
export function brushSpans(b: Brush): [number, number, number][] {
  const n = Math.max(1, Math.round(b.size))
  const lo = -Math.floor((n - 1) / 2) | 0 // `| 0` turns -0 into 0
  const hi = lo + n - 1
  if (!b.round || n <= 2) {
    const rows: [number, number, number][] = []
    for (let dy = lo; dy <= hi; dy++) rows.push([dy, lo, hi])
    return rows
  }
  const rows: [number, number, number][] = []
  ellipseSpans(lo, lo, hi, hi, (y, x0, x1) => rows.push([y, x0, x1]))
  return rows
}

/** Stamps a brush at (x, y). */
export function stamp(spans: [number, number, number][], x: number, y: number, span: SpanFn): void {
  for (const [dy, dx0, dx1] of spans) span(y + dy, x + dx0, x + dx1)
}

/** Normalises two corners into `[xmin, ymin, xmax, ymax]`. */
export function box(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
): [number, number, number, number] {
  return [Math.min(x0, x1), Math.min(y0, y1), Math.max(x0, x1), Math.max(y0, y1)]
}

/**
 * Shape corners from a drag: Shift makes it square, Alt grows it from the start cell as the centre.
 * Returns the inclusive box `[xmin, ymin, xmax, ymax]`.
 */
export function dragBox(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  square: boolean,
  fromCentre: boolean,
): [number, number, number, number] {
  let dx = x1 - x0
  let dy = y1 - y0
  if (square) {
    const d = Math.max(Math.abs(dx), Math.abs(dy))
    dx = (dx < 0 ? -1 : 1) * d
    dy = (dy < 0 ? -1 : 1) * d
  }
  if (fromCentre) return box(x0 - dx, y0 - dy, x0 + dx, y0 + dy)
  return box(x0, y0, x0 + dx, y0 + dy)
}

/** A rectangle, inclusive corners; `thickness` > 0 draws only an outline that wide (inwards). */
export function rectSpans(
  xmin: number,
  ymin: number,
  xmax: number,
  ymax: number,
  thickness: number,
  span: SpanFn,
): void {
  for (let y = ymin; y <= ymax; y++) {
    const edgeRow = thickness <= 0 || y < ymin + thickness || y > ymax - thickness
    if (edgeRow) {
      span(y, xmin, xmax)
    } else {
      span(y, xmin, Math.min(xmax, xmin + thickness - 1))
      span(y, Math.max(xmin, xmax - thickness + 1), xmax)
    }
  }
}

/**
 * The ellipse inscribed in the inclusive box: the cells whose centres lie inside it. Exact for odd
 * and even sizes and symmetric about both axes; a 20 × 20 box gives the disk of diameter 20 that
 * `cfd-core`'s examples draw.
 */
export function ellipseSpans(
  xmin: number,
  ymin: number,
  xmax: number,
  ymax: number,
  span: SpanFn,
): void {
  const cx = (xmin + xmax) / 2
  const cy = (ymin + ymax) / 2
  const a = (xmax - xmin + 1) / 2
  const b = (ymax - ymin + 1) / 2
  for (let y = ymin; y <= ymax; y++) {
    const t = (y - cy) / b
    const hw = a * Math.sqrt(Math.max(0, 1 - t * t))
    // Cells x with |x − cx| ≤ hw; the epsilon keeps exact boundary cases symmetric.
    const x0 = Math.ceil(cx - hw - 1e-9)
    const x1 = Math.floor(cx + hw + 1e-9)
    if (x0 <= x1) span(y, x0, x1)
  }
}

/**
 * An elliptical ring `thickness` cells wide: the cells inside the ellipse of the box and outside
 * the ellipse of the box shrunk by `thickness` on every side.
 */
export function ellipseOutlineSpans(
  xmin: number,
  ymin: number,
  xmax: number,
  ymax: number,
  thickness: number,
  span: SpanFn,
): void {
  const inner = new Map<number, [number, number]>()
  if (xmax - xmin + 1 > 2 * thickness && ymax - ymin + 1 > 2 * thickness) {
    ellipseSpans(
      xmin + thickness,
      ymin + thickness,
      xmax - thickness,
      ymax - thickness,
      (y, a, b) => inner.set(y, [a, b]),
    )
  }
  ellipseSpans(xmin, ymin, xmax, ymax, (y, x0, x1) => {
    const hole = inner.get(y)
    if (!hole) return span(y, x0, x1)
    if (hole[0] > x0) span(y, x0, hole[0] - 1)
    if (hole[1] < x1) span(y, hole[1] + 1, x1)
  })
}

/**
 * Scanline flood fill of the 4-connected region of cells for which `inside` holds, starting at
 * (x, y), within a `width` × `height` grid. `inside` must turn false for a cell once its span has
 * been reported (the caller paints it), which is what stops the fill; no recursion, so it handles
 * a 4096² region.
 */
export function floodFill(
  width: number,
  height: number,
  x: number,
  y: number,
  inside: (x: number, y: number) => boolean,
  span: SpanFn,
): void {
  if (x < 0 || y < 0 || x >= width || y >= height || !inside(x, y)) return
  const stack: number[] = [x, y]
  while (stack.length > 0) {
    const sy = stack.pop()!
    const sx = stack.pop()!
    if (!inside(sx, sy)) continue
    let x0 = sx
    while (x0 > 0 && inside(x0 - 1, sy)) x0--
    let x1 = sx
    while (x1 < width - 1 && inside(x1 + 1, sy)) x1++
    span(sy, x0, x1)
    for (const ny of [sy - 1, sy + 1]) {
      if (ny < 0 || ny >= height) continue
      // One seed per run of inside cells in the neighbouring row.
      let run = false
      for (let nx = x0; nx <= x1; nx++) {
        if (inside(nx, ny)) {
          if (!run) {
            stack.push(nx, ny)
            run = true
          }
        } else {
          run = false
        }
      }
    }
  }
}
