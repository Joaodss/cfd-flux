// Pixel layers of the document being edited. They live outside React (ADR-005): tools write
// into the typed arrays directly, and the renderer uploads the tiles they mark dirty.

/** Cell types of the `cellType` layer (`cfd_core::scene::CellType`). */
export const CellType = {
  Fluid: 0,
  Solid: 1,
  Inlet: 2,
  Outlet: 3,
  HeatSource: 4,
  Empty: 5,
} as const
export type CellType = (typeof CellType)[keyof typeof CellType]

/** Side of the square tiles used for dirty tracking (and, from step 2, the undo history). */
export const TILE = 64

/**
 * The per-cell layers of a scene, row-major from the bottom-left corner:
 * `index = y * width + x` (03 §3).
 */
export class Layers {
  readonly width: number
  readonly height: number
  readonly cellType: Uint8Array
  readonly elementId: Uint16Array
  readonly tilesX: number
  readonly tilesY: number
  /** Bumped on every change: a cheap "has anything changed since" for observers. */
  version = 0
  private readonly dirty: Uint8Array
  private dirtyCount = 0

  constructor(width: number, height: number, cellType?: Uint8Array, elementId?: Uint16Array) {
    const cells = width * height
    if (!(Number.isInteger(width) && Number.isInteger(height) && width > 0 && height > 0)) {
      throw new Error(`invalid grid size ${width} × ${height}`)
    }
    if (cellType && cellType.length !== cells) throw new Error('cellType has the wrong length')
    if (elementId && elementId.length !== cells) throw new Error('elementId has the wrong length')
    this.width = width
    this.height = height
    this.cellType = cellType ?? new Uint8Array(cells)
    this.elementId = elementId ?? new Uint16Array(cells)
    this.tilesX = Math.ceil(width / TILE)
    this.tilesY = Math.ceil(height / TILE)
    this.dirty = new Uint8Array(this.tilesX * this.tilesY)
    this.markAllDirty()
  }

  index(x: number, y: number): number {
    return y * this.width + x
  }

  contains(x: number, y: number): boolean {
    return x >= 0 && y >= 0 && x < this.width && y < this.height
  }

  /** Writes one cell (ignored outside the grid) and marks its tile dirty. */
  set(x: number, y: number, cellType: CellType, elementId: number): void {
    if (!this.contains(x, y)) return
    const i = this.index(x, y)
    this.cellType[i] = cellType
    this.elementId[i] = elementId
    this.markDirty(x, y)
  }

  /** Marks the tile holding cell (x, y) as changed. */
  markDirty(x: number, y: number): void {
    const t = Math.floor(y / TILE) * this.tilesX + Math.floor(x / TILE)
    if (this.dirty[t] === 0) {
      this.dirty[t] = 1
      this.dirtyCount++
    }
    this.version++
  }

  markAllDirty(): void {
    this.dirty.fill(1)
    this.dirtyCount = this.dirty.length
    this.version++
  }

  /**
   * Returns the dirty tiles as cell rectangles `[x0, y0, x1, y1)` and clears them. When most tiles
   * are dirty, a single rectangle covering the grid is returned (one upload instead of many).
   */
  takeDirty(): Rect[] {
    if (this.dirtyCount === 0) return []
    let rects: Rect[]
    if (this.dirtyCount * 2 > this.dirty.length) {
      rects = [{ x0: 0, y0: 0, x1: this.width, y1: this.height }]
    } else {
      rects = []
      for (let t = 0; t < this.dirty.length; t++) {
        if (this.dirty[t] === 0) continue
        const tx = t % this.tilesX
        const ty = Math.floor(t / this.tilesX)
        rects.push({
          x0: tx * TILE,
          y0: ty * TILE,
          x1: Math.min((tx + 1) * TILE, this.width),
          y1: Math.min((ty + 1) * TILE, this.height),
        })
      }
    }
    this.dirty.fill(0)
    this.dirtyCount = 0
    return rects
  }
}

/** A rectangle of cells, `x0 ≤ x < x1`, `y0 ≤ y < y1`. */
export interface Rect {
  x0: number
  y0: number
  x1: number
  y1: number
}
