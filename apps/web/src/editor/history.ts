// Undo/redo by tiles (design proposal §5). An operation writes through a `Stroke`, which copies
// each 64×64 tile the first time it touches it; committing keeps the tiles that really changed.
// Undo *swaps* a stored tile with the layers' current content, so the entry then holds what redo
// needs: one copy per tile, not two.
import type { SceneTable } from '../scene/io.ts'
import { TILE, type CellType, type Layers } from './layers.ts'

/** The content of one tile; a uniform layer is stored as its single value. */
export interface TileData {
  cellType: Uint8Array | number
  elementId: Uint16Array | number
}

/** Bytes a tile takes in the history (a small constant for the object itself). */
function tileBytes(t: TileData): number {
  return (
    64 +
    (typeof t.cellType === 'number' ? 0 : t.cellType.byteLength) +
    (typeof t.elementId === 'number' ? 0 : t.elementId.byteLength)
  )
}

/** The cell rectangle of tile `t`. */
function tileRect(layers: Layers, t: number): [number, number, number, number] {
  const x0 = (t % layers.tilesX) * TILE
  const y0 = Math.floor(t / layers.tilesX) * TILE
  return [x0, y0, Math.min(x0 + TILE, layers.width), Math.min(y0 + TILE, layers.height)]
}

/** Copies a tile out of the layers, compressing uniform layers to one value. */
export function readTile(layers: Layers, t: number): TileData {
  const [x0, y0, x1, y1] = tileRect(layers, t)
  const w = x1 - x0
  const ct = new Uint8Array(w * (y1 - y0))
  const id = new Uint16Array(w * (y1 - y0))
  for (let y = y0; y < y1; y++) {
    const src = y * layers.width + x0
    const dst = (y - y0) * w
    ct.set(layers.cellType.subarray(src, src + w), dst)
    id.set(layers.elementId.subarray(src, src + w), dst)
  }
  return { cellType: uniform(ct) ?? ct, elementId: uniform(id) ?? id }
}

function uniform(a: Uint8Array | Uint16Array): number | null {
  const v = a[0]
  for (let i = 1; i < a.length; i++) if (a[i] !== v) return null
  return v
}

/** Writes a tile back into the layers and marks it dirty. */
export function writeTile(layers: Layers, t: number, data: TileData): void {
  const [x0, y0, x1, y1] = tileRect(layers, t)
  const w = x1 - x0
  for (let y = y0; y < y1; y++) {
    const dst = y * layers.width + x0
    const src = (y - y0) * w
    if (typeof data.cellType === 'number') layers.cellType.fill(data.cellType, dst, dst + w)
    else layers.cellType.set(data.cellType.subarray(src, src + w), dst)
    if (typeof data.elementId === 'number') layers.elementId.fill(data.elementId, dst, dst + w)
    else layers.elementId.set(data.elementId.subarray(src, src + w), dst)
  }
  layers.markDirty(x0, y0)
}

function sameTile(a: TileData, b: TileData): boolean {
  return sameLayer(a.cellType, b.cellType) && sameLayer(a.elementId, b.elementId)
}

function sameLayer(a: Uint8Array | Uint16Array | number, b: Uint8Array | Uint16Array | number) {
  if (typeof a === 'number' || typeof b === 'number') return a === b
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false
  return true
}

/**
 * The cells written by one operation. Tools may `revert()` and redraw any number of times while
 * the pointer moves (live previews of shapes and moved selections); `finish()` turns it into the
 * tiles to keep in the history.
 */
export class Stroke {
  readonly layers: Layers
  private readonly before = new Map<number, TileData>()

  constructor(layers: Layers) {
    this.layers = layers
  }

  /** Writes one cell; cells outside the grid are ignored. */
  write(x: number, y: number, cellType: CellType, elementId: number): void {
    const l = this.layers
    if (x < 0 || y < 0 || x >= l.width || y >= l.height) return
    this.touch(x, y)
    const i = y * l.width + x
    l.cellType[i] = cellType
    l.elementId[i] = elementId
  }

  /** Writes the cells `x0..x1` (inclusive) of row `y`, clipped to the grid. */
  span(y: number, x0: number, x1: number, cellType: CellType, elementId: number): void {
    const l = this.layers
    if (y < 0 || y >= l.height) return
    const a = Math.max(0, x0)
    const b = Math.min(l.width - 1, x1)
    if (a > b) return
    for (let tx = Math.floor(a / TILE); tx <= Math.floor(b / TILE); tx++) this.touch(tx * TILE, y)
    const row = y * l.width
    l.cellType.fill(cellType, row + a, row + b + 1)
    l.elementId.fill(elementId, row + a, row + b + 1)
  }

  /**
   * Copies a `w` × `h` block of cells (row-major, bottom row first) with its bottom-left corner at
   * (x, y), clipped to the grid.
   */
  blit(
    x: number,
    y: number,
    w: number,
    h: number,
    cellType: Uint8Array,
    elementId: Uint16Array,
  ): void {
    const l = this.layers
    const a = Math.max(0, x)
    const b = Math.min(l.width, x + w) // exclusive
    if (a >= b) return
    for (let row = 0; row < h; row++) {
      const yy = y + row
      if (yy < 0 || yy >= l.height) continue
      for (let tx = Math.floor(a / TILE); tx <= Math.floor((b - 1) / TILE); tx++)
        this.touch(tx * TILE, yy)
      const src = row * w + (a - x)
      const dst = yy * l.width + a
      l.cellType.set(cellType.subarray(src, src + b - a), dst)
      l.elementId.set(elementId.subarray(src, src + b - a), dst)
    }
  }

  /** Snapshots the tile of (x, y) before its first write, and marks it dirty for the renderer. */
  private touch(x: number, y: number): void {
    const l = this.layers
    const t = Math.floor(y / TILE) * l.tilesX + Math.floor(x / TILE)
    if (!this.before.has(t)) this.before.set(t, readTile(l, t))
    l.markDirty(x, y)
  }

  /** Restores every touched tile to its content before the stroke (the stroke stays usable). */
  revert(): void {
    for (const [t, data] of this.before) writeTile(this.layers, t, data)
  }

  /** The touched tiles whose content changed, with their content before the stroke. */
  finish(): Map<number, TileData> {
    const changed = new Map<number, TileData>()
    for (const [t, data] of this.before) {
      if (!sameTile(data, readTile(this.layers, t))) changed.set(t, data)
    }
    this.before.clear()
    return changed
  }
}

/** One undoable operation: pixel tiles, scene-table changes, or both. */
export interface Entry {
  label: string
  tiles: Map<number, TileData>
  table?: { before: SceneTable; after: SceneTable }
}

export const HISTORY_BUDGET_BYTES = 256 * 2 ** 20
export const HISTORY_MAX_ENTRIES = 200

function entryBytes(e: Entry): number {
  let n = 256
  for (const t of e.tiles.values()) n += tileBytes(t)
  return n
}

/** The undo and redo stacks of one document. */
export class History {
  private readonly entries: Entry[] = []
  /** Entries before `cursor` are undoable; from `cursor` on, redoable. */
  private cursor = 0
  private bytes = 0
  private readonly budget: number
  private readonly maxEntries: number

  constructor(budget = HISTORY_BUDGET_BYTES, maxEntries = HISTORY_MAX_ENTRIES) {
    this.budget = budget
    this.maxEntries = maxEntries
  }

  get canUndo(): boolean {
    return this.cursor > 0
  }

  get canRedo(): boolean {
    return this.cursor < this.entries.length
  }

  get undoLabel(): string | null {
    return this.canUndo ? this.entries[this.cursor - 1].label : null
  }

  get redoLabel(): string | null {
    return this.canRedo ? this.entries[this.cursor].label : null
  }

  get size(): { entries: number; bytes: number } {
    return { entries: this.entries.length, bytes: this.bytes }
  }

  /** Adds an operation, dropping the redo stack and, over budget, the oldest entries. */
  push(entry: Entry): void {
    for (const e of this.entries.splice(this.cursor)) this.bytes -= entryBytes(e)
    this.entries.push(entry)
    this.bytes += entryBytes(entry)
    this.cursor = this.entries.length
    // Always keep the newest entry, even if it alone exceeds the budget.
    while (
      this.entries.length > 1 &&
      (this.bytes > this.budget || this.entries.length > this.maxEntries)
    ) {
      this.bytes -= entryBytes(this.entries.shift()!)
      this.cursor--
    }
  }

  /**
   * Undoes the last operation on `layers`; returns its entry (whose `table.before` the caller
   * restores), or null when there is nothing to undo.
   */
  undo(layers: Layers): Entry | null {
    if (!this.canUndo) return null
    const e = this.entries[--this.cursor]
    this.swap(e, layers)
    return e
  }

  /** Redoes the next operation; the caller applies `table.after`. */
  redo(layers: Layers): Entry | null {
    if (!this.canRedo) return null
    const e = this.entries[this.cursor++]
    this.swap(e, layers)
    return e
  }

  private swap(e: Entry, layers: Layers): void {
    this.bytes -= entryBytes(e)
    for (const [t, data] of e.tiles) {
      const current = readTile(layers, t)
      writeTile(layers, t, data)
      e.tiles.set(t, current)
    }
    this.bytes += entryBytes(e)
  }
}
