// The drawing tools (design proposal §4). Each tool turns pointer events, in cell coordinates,
// into writes through a `Stroke` and hands each finished operation to `ctx.commit` for the
// history. Shapes are drawn live into the layers and redrawn on every move, so what you see while
// dragging is exactly the cells you get.
import { Stroke, type Entry } from '../history.ts'
import type { CellType, Layers } from '../layers.ts'
import {
  brushSpans,
  dragBox,
  ellipseOutlineSpans,
  ellipseSpans,
  floodFill,
  line,
  rectSpans,
  snapAngle,
  stamp,
  type Brush,
} from './raster.ts'

export type ToolId =
  'hand' | 'pencil' | 'eraser' | 'line' | 'rect' | 'ellipse' | 'bucket' | 'select'

/** What a tool writes into a cell: its type and element id (0 for fluid and empty). */
export interface Paint {
  cellType: CellType
  elementId: number
}

export interface Modifiers {
  shift: boolean
  alt: boolean
}

/** The operation a tool produced, for the history. */
export type Operation = Pick<Entry, 'label' | 'tiles'>

/** Everything a tool reads from the editor, captured when a drag (or a key command) starts. */
export interface ToolContext {
  layers: Layers
  paint: Paint
  /** What the eraser and cleared selections paint: fluid, or empty in a free-surface scene. */
  background: Paint
  brush: Brush
  /** Rectangles and ellipses: filled, or an outline as wide as the brush. */
  filled: boolean
  /** Records a finished operation in the history. */
  commit(op: Operation): void
}

export interface DrawTool {
  down(p: [number, number], m: Modifiers, ctx: ToolContext): void
  move(p: [number, number], m: Modifiers): void
  up(p: [number, number], m: Modifiers): void
  /** Abandons the current drag, restoring its cells (Escape). */
  cancel(): void
  /** Size in cells of the shape being dragged, for the status bar. */
  readonly size?: [number, number] | null
}

/** Commits a stroke's changed tiles, if any. */
export function commitStroke(stroke: Stroke, label: string, ctx: ToolContext): void {
  const tiles = stroke.finish()
  if (tiles.size > 0) ctx.commit({ label, tiles })
}

/** Pencil and eraser: the brush stamped along the pointer's path. */
export class Freehand implements DrawTool {
  private stroke: Stroke | null = null
  private ctx: ToolContext | null = null
  private last: [number, number] = [0, 0]
  private spans: [number, number, number][] = []
  private paint: Paint = { cellType: 0, elementId: 0 }
  private readonly eraser: boolean

  constructor(eraser: boolean) {
    this.eraser = eraser
  }

  down(p: [number, number], _m: Modifiers, ctx: ToolContext): void {
    this.stroke = new Stroke(ctx.layers)
    this.ctx = ctx
    this.spans = brushSpans(ctx.brush)
    this.paint = this.eraser ? ctx.background : ctx.paint
    this.last = p
    this.stampAt(p[0], p[1])
  }

  move(p: [number, number]): void {
    if (!this.stroke) return
    // Stamp along the segment, so fast pointer moves leave no gaps.
    line(this.last[0], this.last[1], p[0], p[1], (x, y) => this.stampAt(x, y))
    this.last = p
  }

  up(): void {
    if (this.stroke) commitStroke(this.stroke, this.eraser ? 'Eraser' : 'Pencil', this.ctx!)
    this.stroke = null
  }

  cancel(): void {
    this.stroke?.revert()
    this.stroke = null
  }

  private stampAt(x: number, y: number): void {
    const s = this.stroke!
    const { cellType, elementId } = this.paint
    stamp(this.spans, x, y, (yy, x0, x1) => s.span(yy, x0, x1, cellType, elementId))
  }
}

type ShapeKind = 'line' | 'rect' | 'ellipse'
const SHAPE_LABELS: Record<ShapeKind, string> = {
  line: 'Line',
  rect: 'Rectangle',
  ellipse: 'Ellipse',
}

/**
 * Line (Shift: 0/45/90°), rectangle and ellipse (Shift: square/circle, Alt: from the centre),
 * redrawn from the start cell on every move.
 */
export class Shape implements DrawTool {
  private stroke: Stroke | null = null
  private ctx: ToolContext | null = null
  private start: [number, number] = [0, 0]
  private readonly kind: ShapeKind
  size: [number, number] | null = null

  constructor(kind: ShapeKind) {
    this.kind = kind
  }

  down(p: [number, number], m: Modifiers, ctx: ToolContext): void {
    this.stroke = new Stroke(ctx.layers)
    this.ctx = ctx
    this.start = p
    this.draw(p, m)
  }

  move(p: [number, number], m: Modifiers): void {
    if (!this.stroke) return
    this.stroke.revert()
    this.draw(p, m)
  }

  up(p: [number, number], m: Modifiers): void {
    if (!this.stroke) return
    this.move(p, m)
    commitStroke(this.stroke, SHAPE_LABELS[this.kind], this.ctx!)
    this.stroke = null
    this.size = null
  }

  cancel(): void {
    this.stroke?.revert()
    this.stroke = null
    this.size = null
  }

  private draw(p: [number, number], m: Modifiers): void {
    const s = this.stroke!
    const { paint, brush, filled } = this.ctx!
    const span = (y: number, x0: number, x1: number) =>
      s.span(y, x0, x1, paint.cellType, paint.elementId)
    const [x0, y0] = this.start
    if (this.kind === 'line') {
      const [x1, y1] = m.shift ? snapAngle(x0, y0, p[0], p[1]) : p
      const spans = brushSpans(brush)
      line(x0, y0, x1, y1, (x, y) => stamp(spans, x, y, span))
      this.size = [Math.abs(x1 - x0) + 1, Math.abs(y1 - y0) + 1]
      return
    }
    const [xmin, ymin, xmax, ymax] = dragBox(x0, y0, p[0], p[1], m.shift, m.alt)
    const thickness = filled ? 0 : Math.max(1, Math.round(brush.size))
    if (this.kind === 'rect') rectSpans(xmin, ymin, xmax, ymax, thickness, span)
    else if (filled) ellipseSpans(xmin, ymin, xmax, ymax, span)
    else ellipseOutlineSpans(xmin, ymin, xmax, ymax, thickness, span)
    this.size = [xmax - xmin + 1, ymax - ymin + 1]
  }
}

/** Bucket: fills the 4-connected region of cells equal (type and element) to the clicked one. */
export class Bucket implements DrawTool {
  down(p: [number, number], _m: Modifiers, ctx: ToolContext): void {
    const l = ctx.layers
    if (!l.contains(p[0], p[1])) return
    const i = l.index(p[0], p[1])
    const ct = l.cellType[i]
    const id = l.elementId[i]
    const { cellType, elementId } = ctx.paint
    if (ct === cellType && id === elementId) return
    const s = new Stroke(l)
    const w = l.width
    floodFill(
      w,
      l.height,
      p[0],
      p[1],
      (x, y) => l.cellType[y * w + x] === ct && l.elementId[y * w + x] === id,
      (y, x0, x1) => s.span(y, x0, x1, cellType, elementId),
    )
    commitStroke(s, 'Fill', ctx)
  }

  move(): void {}
  up(): void {}
  cancel(): void {}
}
