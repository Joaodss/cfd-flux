import { describe, expect, it } from 'vitest'
import { History } from '../src/editor/history.ts'
import { CellType, Layers } from '../src/editor/layers.ts'
import { SelectTool, type Selection } from '../src/editor/tools/select.ts'
import {
  Bucket,
  Freehand,
  Shape,
  type DrawTool,
  type Modifiers,
  type ToolContext,
} from '../src/editor/tools/tools.ts'

const NONE: Modifiers = { shift: false, alt: false }
const WALL = { cellType: CellType.Solid, elementId: 1 }

function setup(w = 32, h = 16, filled = true) {
  const layers = new Layers(w, h)
  const history = new History()
  const ctx: ToolContext = {
    layers,
    paint: WALL,
    background: { cellType: CellType.Fluid, elementId: 0 },
    brush: { size: 1, round: false },
    filled,
    commit: (op) => history.push(op),
  }
  return { layers, history, ctx }
}

function drag(tool: DrawTool, ctx: ToolContext, path: [number, number][], m = NONE) {
  tool.down(path[0], m, ctx)
  for (const p of path.slice(1)) tool.move(p, m)
  tool.up(path.at(-1)!, m)
}

/** Picture of the layers, top row first: `.` fluid, `#` solid, other types by number. */
function picture(l: Layers): string {
  const rows: string[] = []
  for (let y = l.height - 1; y >= 0; y--) {
    let r = ''
    for (let x = 0; x < l.width; x++) {
      const t = l.cellType[l.index(x, y)]
      r += t === CellType.Fluid ? '.' : t === CellType.Solid ? '#' : String(t)
    }
    rows.push(r)
  }
  return rows.join('\n')
}

const count = (l: Layers, ct: number) => l.cellType.reduce((n, v) => n + (v === ct ? 1 : 0), 0)

describe('pencil and eraser', () => {
  it('leave no gaps between distant pointer events, and are one undo entry per stroke', () => {
    const { layers, history, ctx } = setup()
    drag(new Freehand(false), ctx, [
      [0, 0],
      [20, 0],
    ])
    expect(count(layers, CellType.Solid)).toBe(21)
    expect(layers.elementId[layers.index(10, 0)]).toBe(1)
    expect(history.size.entries).toBe(1)
    drag(new Freehand(true), ctx, [
      [5, 0],
      [9, 0],
    ])
    expect(count(layers, CellType.Solid)).toBe(16)
    expect(layers.elementId[layers.index(7, 0)]).toBe(0)
    history.undo(layers)
    expect(count(layers, CellType.Solid)).toBe(21)
  })

  it('a stroke that changes nothing adds no history entry', () => {
    const { history, ctx } = setup()
    drag(new Freehand(true), ctx, [
      [1, 1],
      [5, 5],
    ])
    expect(history.size.entries).toBe(0)
  })
})

describe('shapes', () => {
  it('preview live and keep only the final shape', () => {
    const { layers, history, ctx } = setup()
    drag(new Shape('rect'), ctx, [
      [2, 2],
      [20, 12],
      [5, 4],
    ])
    expect(count(layers, CellType.Solid)).toBe(4 * 3)
    expect(history.size.entries).toBe(1)
    expect(history.undoLabel).toBe('Rectangle')
  })

  it('draws outlines as wide as the brush', () => {
    const { layers, ctx } = setup(8, 6, false)
    ctx.brush = { size: 1, round: false }
    drag(new Shape('rect'), ctx, [
      [1, 1],
      [6, 4],
    ])
    expect(picture(layers)).toBe(
      ['........', '.######.', '.#....#.', '.#....#.', '.######.', '........'].join('\n'),
    )
  })

  it('Shift makes a circle from a drag; the cylinder is a 20 × 20 drag', () => {
    const { layers, ctx } = setup(64, 64)
    const tool = new Shape('ellipse')
    tool.down([31, 31], { shift: true, alt: false }, ctx)
    tool.move([50, 45], { shift: true, alt: false })
    expect(tool.size).toEqual([20, 20])
    tool.up([50, 45], { shift: true, alt: false })
    expect(count(layers, CellType.Solid)).toBe(316)
  })
})

describe('bucket', () => {
  it('fills the region of the clicked cell, bounded by other cells', () => {
    const { layers, ctx } = setup(10, 6, false)
    drag(new Shape('rect'), ctx, [
      [2, 1],
      [7, 4],
    ])
    ctx.paint = { cellType: CellType.Inlet, elementId: 2 }
    drag(new Bucket(), ctx, [[4, 2]])
    expect(picture(layers)).toBe(
      ['..........', '..######..', '..#2222#..', '..#2222#..', '..######..', '..........'].join(
        '\n',
      ),
    )
  })

  it('does nothing when the region already has the paint', () => {
    const { history, ctx } = setup()
    ctx.paint = { cellType: CellType.Fluid, elementId: 0 }
    drag(new Bucket(), ctx, [[3, 3]])
    expect(history.size.entries).toBe(0)
  })
})

describe('select', () => {
  function withBlock() {
    const s = setup(12, 6)
    drag(new Shape('rect'), s.ctx, [
      [1, 1],
      [2, 2],
    ])
    const changes: [Selection | null, boolean][] = []
    const tool = new SelectTool((sel, floating) => changes.push([sel, floating]))
    return { ...s, tool, changes }
  }

  it('moves a selection as one undo entry, leaving the background behind', () => {
    const { layers, history, ctx, tool } = withBlock()
    drag(tool, ctx, [
      [0, 0],
      [3, 3],
    ])
    expect(tool.current).toEqual([0, 0, 3, 3])
    drag(tool, ctx, [
      [1, 1],
      [6, 2],
    ])
    expect(tool.isFloating).toBe(true)
    tool.drop()
    expect(tool.current).toEqual([5, 1, 8, 4])
    expect(picture(layers)).toBe(
      [
        '............',
        '............',
        '......##....',
        '......##....',
        '............',
        '............',
      ].join('\n'),
    )
    expect(history.undoLabel).toBe('Move')
    history.undo(layers)
    expect(layers.cellType[layers.index(1, 1)]).toBe(CellType.Solid)
    expect(count(layers, CellType.Solid)).toBe(4)
  })

  it('copies with Alt+drag and puts everything back on Escape', () => {
    const { layers, ctx, tool } = withBlock()
    drag(tool, ctx, [
      [1, 1],
      [2, 2],
    ])
    drag(
      tool,
      ctx,
      [
        [1, 1],
        [8, 1],
      ],
      { shift: false, alt: true },
    )
    expect(count(layers, CellType.Solid)).toBe(8)
    tool.cancel()
    expect(count(layers, CellType.Solid)).toBe(4)
    expect(tool.isFloating).toBe(false)
  })

  it('drops a floating selection when clicking outside it', () => {
    const { layers, history, ctx, tool } = withBlock()
    drag(tool, ctx, [
      [1, 1],
      [2, 2],
    ])
    drag(
      tool,
      ctx,
      [
        [1, 1],
        [5, 1],
      ],
      { shift: false, alt: true },
    )
    drag(tool, ctx, [[11, 5]]) // click outside: drop, then deselect
    expect(tool.isFloating).toBe(false)
    expect(tool.current).toBeNull()
    expect(history.undoLabel).toBe('Copy')
    expect(count(layers, CellType.Solid)).toBe(8)
  })

  it('deletes, nudges, copies and pastes', () => {
    const { layers, history, ctx, tool } = withBlock()
    drag(tool, ctx, [
      [1, 1],
      [2, 2],
    ])
    expect(tool.copy(ctx)).toBe(true)
    tool.nudge(1, 0, ctx)
    tool.nudge(0, 1, ctx)
    tool.drop()
    expect(layers.cellType[layers.index(3, 3)]).toBe(CellType.Solid)
    expect(count(layers, CellType.Solid)).toBe(4)
    tool.paste([8, 0], ctx)
    tool.drop()
    expect(count(layers, CellType.Solid)).toBe(8)
    expect(tool.current).toEqual([8, 0, 9, 1])
    tool.clear(ctx)
    expect(count(layers, CellType.Solid)).toBe(4)
    expect(history.undoLabel).toBe('Delete')
  })

  it('clips a block dropped across the grid edge', () => {
    const { layers, ctx, tool } = withBlock()
    drag(tool, ctx, [
      [1, 1],
      [2, 2],
    ])
    drag(tool, ctx, [
      [1, 1],
      [11, 5],
    ])
    tool.drop()
    expect(tool.current).toEqual([11, 5, 11, 5])
    expect(count(layers, CellType.Solid)).toBe(1)
  })
})
