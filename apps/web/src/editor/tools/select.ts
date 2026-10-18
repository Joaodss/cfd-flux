// The select tool: a rectangular selection that can be moved (drag), copied (Alt+drag, or
// Ctrl+C / Ctrl+V), cleared (Delete) and nudged (arrows). Moving lifts the cells into a *floating*
// selection drawn live into the layers through one stroke; it is committed as a single undo entry
// when it is dropped (Enter, a click outside it, another tool) and Escape puts everything back.
import { Stroke } from '../history.ts'
import { box } from './raster.ts'
import { commitStroke, type DrawTool, type Modifiers, type ToolContext } from './tools.ts'

/** An inclusive cell rectangle `[xmin, ymin, xmax, ymax]`. */
export type Selection = [number, number, number, number]

/** A block of cells (row-major, bottom row first). */
export interface Block {
  width: number
  height: number
  cellType: Uint8Array
  elementId: Uint16Array
}

interface Floating {
  stroke: Stroke
  block: Block
  /** Bottom-left cell of the block's current position. */
  x: number
  y: number
  /** Where the block was lifted from (cleared to the background), or null for a copy/paste. */
  source: Selection | null
  label: string
}

/** Notified whenever the selection or its floating state changes (the store mirrors it). */
export type SelectionListener = (selection: Selection | null, floating: boolean) => void

function inside(s: Selection | null, [x, y]: [number, number]): boolean {
  return s !== null && x >= s[0] && x <= s[2] && y >= s[1] && y <= s[3]
}

export class SelectTool implements DrawTool {
  private selection: Selection | null = null
  private floating: Floating | null = null
  private ctx: ToolContext | null = null
  private mode: 'marquee' | 'move' | null = null
  private start: [number, number] = [0, 0]
  private origin: [number, number] = [0, 0]
  private moved = false
  private clipboard: Block | null = null
  private readonly listener: SelectionListener

  constructor(listener: SelectionListener) {
    this.listener = listener
  }

  get current(): Selection | null {
    return this.selection
  }

  get isFloating(): boolean {
    return this.floating !== null
  }

  get size(): [number, number] | null {
    const s = this.selection
    return this.mode && s ? [s[2] - s[0] + 1, s[3] - s[1] + 1] : null
  }

  down(p: [number, number], m: Modifiers, ctx: ToolContext): void {
    this.ctx = ctx
    this.start = p
    this.moved = false
    if (this.floating && inside(this.selection, p)) {
      this.mode = 'move'
    } else {
      this.drop()
      if (inside(this.selection, p)) {
        this.lift(m.alt)
        this.mode = 'move'
      } else {
        this.mode = 'marquee'
        this.setSelection(this.clip(p[0], p[1], p[0], p[1]))
      }
    }
    if (this.floating) this.origin = [this.floating.x, this.floating.y]
  }

  move(p: [number, number]): void {
    if (p[0] !== this.start[0] || p[1] !== this.start[1]) this.moved = true
    if (this.mode === 'marquee') {
      this.setSelection(this.clip(this.start[0], this.start[1], p[0], p[1]))
    } else if (this.mode === 'move') {
      this.place(this.origin[0] + p[0] - this.start[0], this.origin[1] + p[1] - this.start[1])
    }
  }

  up(p: [number, number]): void {
    this.move(p)
    // A click without a drag deselects.
    if (this.mode === 'marquee' && !this.moved) this.setSelection(null)
    this.mode = null
  }

  /** Escape: puts a floating selection back, or deselects. */
  cancel(): void {
    this.mode = null
    if (this.floating) {
      const f = this.floating
      f.stroke.revert()
      this.floating = null
      this.setSelection(f.source ?? null)
    } else {
      this.setSelection(null)
    }
  }

  /** Commits a floating selection as one operation (it stays selected where it was dropped). */
  drop(): void {
    const f = this.floating
    if (!f) return
    this.floating = null
    commitStroke(f.stroke, f.label, this.ctx!)
    // Cells dropped outside the grid are gone: keep the selection on what remains.
    const s = this.selection
    this.setSelection(s && this.clip(s[0], s[1], s[2], s[3]))
  }

  /** The document was replaced: forget everything without touching the (old) layers. */
  reset(): void {
    this.floating = null
    this.mode = null
    this.setSelection(null)
  }

  selectAll(ctx: ToolContext): void {
    this.ctx = ctx
    this.drop()
    this.setSelection([0, 0, ctx.layers.width - 1, ctx.layers.height - 1])
  }

  /** Delete: clears the selected cells to the background (a floating paste is just discarded). */
  clear(ctx: ToolContext): void {
    this.ctx = ctx
    const f = this.floating
    if (f) {
      f.stroke.revert()
      this.floating = null
      const s = f.source
      if (s) this.fill(f.stroke, s)
      commitStroke(f.stroke, 'Delete', ctx)
      this.setSelection(null)
    } else if (this.selection) {
      const stroke = new Stroke(ctx.layers)
      this.fill(stroke, this.selection)
      commitStroke(stroke, 'Delete', ctx)
    }
  }

  /** Arrow keys: moves the selection by (dx, dy) cells, lifting it first if needed. */
  nudge(dx: number, dy: number, ctx: ToolContext): void {
    this.ctx = ctx
    if (!this.selection) return
    if (!this.floating) this.lift(false)
    const f = this.floating!
    this.place(f.x + dx, f.y + dy)
  }

  copy(ctx: ToolContext): boolean {
    this.ctx = ctx
    if (this.floating) this.clipboard = this.floating.block
    else if (this.selection) this.clipboard = this.read(this.selection)
    return this.clipboard !== null
  }

  cut(ctx: ToolContext): void {
    if (this.copy(ctx)) this.clear(ctx)
  }

  /** Pastes the clipboard as a floating selection with its bottom-left cell at `at`. */
  paste(at: [number, number], ctx: ToolContext): boolean {
    if (!this.clipboard) return false
    this.float(this.clipboard, at, 'Paste', ctx)
    return true
  }

  /** Places any block (a paste, an imported image) as a floating selection. */
  float(block: Block, at: [number, number], label: string, ctx: ToolContext): void {
    this.ctx = ctx
    this.drop()
    this.floating = { stroke: new Stroke(ctx.layers), block, x: 0, y: 0, source: null, label }
    this.place(at[0], at[1])
  }

  // ---- internals ------------------------------------------------------------------------

  private lift(keepSource: boolean): void {
    const s = this.selection!
    this.floating = {
      stroke: new Stroke(this.ctx!.layers),
      block: this.read(s),
      x: s[0],
      y: s[1],
      source: keepSource ? null : s,
      label: keepSource ? 'Copy' : 'Move',
    }
    this.place(s[0], s[1])
  }

  /** Redraws the floating block at (x, y): revert, clear its source, copy the block. */
  private place(x: number, y: number): void {
    const f = this.floating!
    f.stroke.revert()
    if (f.source) this.fill(f.stroke, f.source)
    const b = f.block
    f.stroke.blit(x, y, b.width, b.height, b.cellType, b.elementId)
    f.x = x
    f.y = y
    this.setSelection([x, y, x + b.width - 1, y + b.height - 1])
  }

  private fill(stroke: Stroke, s: Selection): void {
    const { cellType, elementId } = this.ctx!.background
    for (let y = s[1]; y <= s[3]; y++) stroke.span(y, s[0], s[2], cellType, elementId)
  }

  private read(s: Selection): Block {
    const l = this.ctx!.layers
    const width = s[2] - s[0] + 1
    const height = s[3] - s[1] + 1
    const cellType = new Uint8Array(width * height)
    const elementId = new Uint16Array(width * height)
    for (let row = 0; row < height; row++) {
      const src = (s[1] + row) * l.width + s[0]
      cellType.set(l.cellType.subarray(src, src + width), row * width)
      elementId.set(l.elementId.subarray(src, src + width), row * width)
    }
    return { width, height, cellType, elementId }
  }

  /** The box between two cells, clipped to the grid (null if it lies outside). */
  private clip(x0: number, y0: number, x1: number, y1: number): Selection | null {
    const l = this.ctx!.layers
    const [a, b, c, d] = box(x0, y0, x1, y1)
    const s: Selection = [
      Math.max(0, a),
      Math.max(0, b),
      Math.min(l.width - 1, c),
      Math.min(l.height - 1, d),
    ]
    return s[0] <= s[2] && s[1] <= s[3] ? s : null
  }

  private setSelection(s: Selection | null): void {
    this.selection = s
    this.notify()
  }

  private notify(): void {
    this.listener(this.selection, this.floating !== null)
  }
}
