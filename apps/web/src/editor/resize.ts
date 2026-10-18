// Changing the grid (design proposal §6, Q8): resizing keeps the drawing anchored at the
// bottom-left cell, cropping or extending with the background; scaling refines the drawing by an
// integer factor (each cell becomes k × k cells, the cell size divided by k).
import type { SceneTable } from '../scene/io.ts'
import { Layers } from './layers.ts'
import type { Paint } from './tools/tools.ts'

/** Largest grid side the solver accepts (`cfd_core::validate::MAX_GRID_SIDE`). */
export const MAX_GRID_SIDE = 4096

/** The layers cropped or extended to `width` × `height`, anchored at the bottom-left cell. */
export function resizeLayers(l: Layers, width: number, height: number, bg: Paint): Layers {
  const out = new Layers(width, height)
  out.cellType.fill(bg.cellType)
  out.elementId.fill(bg.elementId)
  const w = Math.min(width, l.width)
  for (let y = 0; y < Math.min(height, l.height); y++) {
    out.cellType.set(l.cellType.subarray(y * l.width, y * l.width + w), y * width)
    out.elementId.set(l.elementId.subarray(y * l.width, y * l.width + w), y * width)
  }
  return out
}

/** Each cell becomes `k` × `k` cells. */
export function scaleLayers(l: Layers, k: number): Layers {
  const out = new Layers(l.width * k, l.height * k)
  for (let y = 0; y < out.height; y++) {
    const src = Math.floor(y / k) * l.width
    const dst = y * out.width
    for (let x = 0; x < out.width; x++) {
      const i = src + Math.floor(x / k)
      out.cellType[dst + x] = l.cellType[i]
      out.elementId[dst + x] = l.elementId[i]
    }
  }
  return out
}

/** The table after a resize: new grid size, and probes outside the new grid removed. */
export function resizeTable(t: SceneTable, width: number, height: number): SceneTable {
  return {
    ...t,
    grid: { ...t.grid, width, height },
    probes: (t.probes ?? []).filter((p) => p.position[0] < width && p.position[1] < height),
  }
}

/** The table after scaling by `k`: finer cells, probes moved to the middle of their new block. */
export function scaleTable(t: SceneTable, k: number): SceneTable {
  const mid = Math.floor(k / 2)
  return {
    ...t,
    grid: {
      ...t.grid,
      width: t.grid.width * k,
      height: t.grid.height * k,
      cellSize: t.grid.cellSize / k,
    },
    probes: (t.probes ?? []).map((p) => ({
      ...p,
      position: [p.position[0] * k + mid, p.position[1] * k + mid] as [number, number],
    })),
  }
}
