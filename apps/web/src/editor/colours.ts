// Colours of cells and elements on the canvas.
import type { Element } from '../generated/scene.ts'
import { CellType } from './layers.ts'

export type ElementKind = Element['kind']

/** RGBA colours, 0–255 per channel. */
export type Rgba = readonly [number, number, number, number]

/** Everything the renderer colours. */
export interface Colours {
  /** Indexed by cell type (fluid, solid, inlet, outlet, heat source, empty). */
  cellTypes: readonly Rgba[]
  /** Element id → colour; ids not listed fall back to their cell type's colour. */
  elements: ReadonlyMap<number, Rgba>
  background: Rgba
  grid: Rgba
  border: Rgba
}

/** Base colour per cell type, indexed by `CellType`. */
export const CELL_COLOURS: readonly Rgba[] = [
  [232, 240, 250, 255], // fluid
  [71, 85, 105, 255], // solid
  [22, 163, 74, 255], // inlet
  [220, 38, 38, 255], // outlet
  [234, 88, 12, 255], // heat source
  [250, 250, 250, 255], // empty
]

export const KIND_CELL: Record<ElementKind, CellType> = {
  wall: CellType.Solid,
  inlet: CellType.Inlet,
  outlet: CellType.Outlet,
  heatSource: CellType.HeatSource,
}

/** Hue (degrees), saturation and lightness range of each kind's elements. */
const KIND_HSL: Record<ElementKind, [number, number, number, number]> = {
  wall: [215, 0.16, 0.22, 0.5],
  inlet: [142, 0.7, 0.3, 0.55],
  outlet: [0, 0.72, 0.38, 0.62],
  heatSource: [24, 0.9, 0.42, 0.6],
}

function hsl(h: number, s: number, l: number): Rgba {
  // Standard HSL → RGB.
  const a = s * Math.min(l, 1 - l)
  const f = (n: number) => {
    const k = (n + h / 30) % 12
    return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))))
  }
  return [f(0), f(8), f(4), 255]
}

/**
 * The colour of the `ordinal`-th element (0-based) of a kind: same hue, lightness stepping through
 * the kind's range so neighbouring elements of one kind stay distinguishable.
 */
export function elementColour(kind: ElementKind, ordinal: number): Rgba {
  const [h, s, l0, l1] = KIND_HSL[kind]
  const steps = [0, 0.5, 0.25, 0.75, 0.125, 0.625, 0.375, 0.875]
  const t = steps[ordinal % steps.length]
  return hsl(h, s, l0 + (l1 - l0) * t)
}

const cache = new WeakMap<readonly Element[], Colours>()

/** The renderer's colours for a scene's elements (cached per elements array: it is immutable). */
export function coloursFor(elements: readonly Element[]): Colours {
  let c = cache.get(elements)
  if (!c) {
    const ordinals: Partial<Record<ElementKind, number>> = {}
    const map = new Map<number, Rgba>()
    for (const e of elements) {
      const n = ordinals[e.kind] ?? 0
      ordinals[e.kind] = n + 1
      map.set(e.id, elementColour(e.kind, n))
    }
    c = {
      cellTypes: CELL_COLOURS,
      elements: map,
      background: [38, 41, 48, 255],
      grid: [0, 0, 0, 56],
      border: [148, 163, 184, 255],
    }
    cache.set(elements, c)
  }
  return c
}

export function rgbaCss([r, g, b]: Rgba): string {
  return `rgb(${r} ${g} ${b})`
}
