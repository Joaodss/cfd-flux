// The four domain edges (Q14 option B): what each one is along its length — the scene's edge kind
// (`grid.edges`: wall or periodic) where nothing is drawn on the edge cells, else the drawn wall,
// inlet or outlet — and the "Make inlet" / "Make outlet" action, which paints the edge cells.
import type { DomainEdges } from '../generated/scene.ts'
import type { Stroke } from './history.ts'
import { CellType, type Layers } from './layers.ts'
import type { Paint } from './tools/tools.ts'

export const SIDES = ['left', 'right', 'bottom', 'top'] as const
export type Side = (typeof SIDES)[number]

export const SIDE_NAMES: Record<Side, string> = {
  left: 'Left',
  right: 'Right',
  bottom: 'Bottom',
  top: 'Top',
}

/** The unit vector into the domain from each edge (an edge inlet blows along it). */
export const INWARD: Record<Side, [number, number]> = {
  left: [1, 0],
  right: [-1, 0],
  bottom: [0, 1],
  top: [0, -1],
}

/** The edges of a scene without `grid.edges`: walls all around (03 §3). */
export const WALL_EDGES: DomainEdges = { left: 'wall', right: 'wall', bottom: 'wall', top: 'wall' }

/** What the boundary is along a stretch of an edge. */
export type EdgeKindShown = 'wall' | 'periodic' | 'inlet' | 'outlet'

/** A stretch of equal boundary along an edge: cells `start ≤ i < end` (see `edgeCell`). */
export interface EdgeRun {
  kind: EdgeKindShown
  /** The drawn element (0 where nothing is drawn: the scene's edge kind applies). */
  elementId: number
  start: number
  end: number
}

/** Number of cells along an edge. */
export function edgeLength(layers: Layers, side: Side): number {
  return side === 'left' || side === 'right' ? layers.height : layers.width
}

/** The `i`-th cell along an edge: left and right go bottom to top, bottom and top left to right. */
export function edgeCell(layers: Layers, side: Side, i: number): [number, number] {
  switch (side) {
    case 'left':
      return [0, i]
    case 'right':
      return [layers.width - 1, i]
    case 'bottom':
      return [i, 0]
    case 'top':
      return [i, layers.height - 1]
  }
}

/**
 * The boundary along an edge as runs: an inlet or outlet cell is that element; a drawn wall or
 * heat source is a wall; a fluid or empty cell leaves the scene's edge kind in place.
 */
export function edgeRuns(layers: Layers, edges: DomainEdges, side: Side): EdgeRun[] {
  const runs: EdgeRun[] = []
  const n = edgeLength(layers, side)
  for (let i = 0; i < n; i++) {
    const c = layers.index(...edgeCell(layers, side, i))
    const ct = layers.cellType[c]
    let kind: EdgeKindShown
    let elementId = layers.elementId[c]
    if (ct === CellType.Inlet) kind = 'inlet'
    else if (ct === CellType.Outlet) kind = 'outlet'
    else if (ct === CellType.Solid || ct === CellType.HeatSource) kind = 'wall'
    else {
      kind = edges[side]
      elementId = 0
    }
    const last = runs[runs.length - 1]
    if (last && last.kind === kind && last.elementId === elementId) last.end = i + 1
    else runs.push({ kind, elementId, start: i, end: i + 1 })
  }
  return runs
}

/**
 * Whether "Make inlet/outlet" repaints an edge cell: fluid, empty, inlet and outlet cells are; drawn
 * walls and heat sources stay (so walls drawn along the neighbouring edges keep their corners).
 */
export function isOpenable(cellType: number): boolean {
  return cellType !== CellType.Solid && cellType !== CellType.HeatSource
}

/** Paints the openable cells of an edge with `paint` (an inlet or outlet element). */
export function paintEdge(stroke: Stroke, side: Side, paint: Paint): void {
  const l = stroke.layers
  const n = edgeLength(l, side)
  for (let i = 0; i < n; i++) {
    const [x, y] = edgeCell(l, side, i)
    if (isOpenable(l.cellType[l.index(x, y)])) stroke.write(x, y, paint.cellType, paint.elementId)
  }
}

/** What the Scene tab shows about an edge and whether its buttons can act. */
export interface EdgeSummary {
  runs: EdgeRun[]
  /** Cells "Make inlet/outlet" would paint. */
  openable: number
  /** The inlet or outlet element covering every openable cell, if there is one. */
  sole: { kind: 'inlet' | 'outlet'; elementId: number } | null
}

export function edgeSummary(layers: Layers, edges: DomainEdges, side: Side): EdgeSummary {
  const runs = edgeRuns(layers, edges, side)
  let openable = 0
  const n = edgeLength(layers, side)
  for (let i = 0; i < n; i++) {
    if (isOpenable(layers.cellType[layers.index(...edgeCell(layers, side, i))])) openable++
  }
  const open = runs.filter((r) => r.kind === 'inlet' || r.kind === 'outlet')
  const covered = open.reduce((sum, r) => sum + r.end - r.start, 0)
  const first = open[0]
  const sole =
    first && covered === openable && open.every((r) => r.elementId === first.elementId)
      ? { kind: first.kind as 'inlet' | 'outlet', elementId: first.elementId }
      : null
  return { runs, openable, sole }
}
