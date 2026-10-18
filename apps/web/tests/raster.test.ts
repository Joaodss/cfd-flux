import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import {
  brushSpans,
  dragBox,
  ellipseOutlineSpans,
  ellipseSpans,
  floodFill,
  line,
  rectSpans,
  snapAngle,
  type SpanFn,
} from '../src/editor/tools/raster.ts'
import { fromScene } from '../src/scene/io.ts'

/** Renders spans into a set of "x,y" keys. */
function cells(draw: (span: SpanFn) => void): Set<string> {
  const s = new Set<string>()
  draw((y, x0, x1) => {
    for (let x = x0; x <= x1; x++) s.add(`${x},${y}`)
  })
  return s
}

/** ASCII picture of cells in a w × h grid, top row first (y up), for readable expectations. */
function picture(set: Set<string>, w: number, h: number): string {
  const rows: string[] = []
  for (let y = h - 1; y >= 0; y--) {
    let r = ''
    for (let x = 0; x < w; x++) r += set.has(`${x},${y}`) ? '#' : '.'
    rows.push(r)
  }
  return rows.join('\n')
}

describe('line', () => {
  it('includes both ends and is 8-connected', () => {
    const pts: [number, number][] = []
    line(0, 0, 5, 2, (x, y) => pts.push([x, y]))
    expect(pts[0]).toEqual([0, 0])
    expect(pts.at(-1)).toEqual([5, 2])
    expect(pts).toHaveLength(6)
    for (let i = 1; i < pts.length; i++) {
      expect(Math.abs(pts[i][0] - pts[i - 1][0])).toBeLessThanOrEqual(1)
      expect(Math.abs(pts[i][1] - pts[i - 1][1])).toBeLessThanOrEqual(1)
    }
  })

  it('draws a single point', () => {
    const pts: [number, number][] = []
    line(3, 4, 3, 4, (x, y) => pts.push([x, y]))
    expect(pts).toEqual([[3, 4]])
  })

  it('snaps to 0, 45 and 90 degrees', () => {
    expect(snapAngle(0, 0, 10, 2)).toEqual([10, 0])
    expect(snapAngle(0, 0, 1, -9)).toEqual([0, -9])
    expect(snapAngle(0, 0, 7, -6)).toEqual([7, -7])
  })
})

describe('brush', () => {
  it('square brushes are centred, even sizes extending towards +x/+y', () => {
    expect(brushSpans({ size: 1, round: false })).toEqual([[0, 0, 0]])
    expect(brushSpans({ size: 2, round: false })).toEqual([
      [0, 0, 1],
      [1, 0, 1],
    ])
    expect(brushSpans({ size: 3, round: false })).toHaveLength(3)
  })

  it('round brushes are disks', () => {
    const s = cells((span) => {
      for (const [dy, a, b] of brushSpans({ size: 5, round: true })) span(dy + 2, a + 2, b + 2)
    })
    expect(picture(s, 5, 5)).toBe(['.###.', '#####', '#####', '#####', '.###.'].join('\n'))
  })
})

describe('shapes', () => {
  it('drag boxes: plain, square (Shift) and from the centre (Alt)', () => {
    expect(dragBox(5, 5, 2, 9, false, false)).toEqual([2, 5, 5, 9])
    expect(dragBox(5, 5, 7, 6, true, false)).toEqual([5, 5, 7, 7])
    expect(dragBox(5, 5, 7, 6, false, true)).toEqual([3, 4, 7, 6])
  })

  it('rectangles: filled and outlined', () => {
    expect(
      picture(
        cells((s) => rectSpans(1, 1, 4, 3, 0, s)),
        6,
        5,
      ),
    ).toBe(['......', '.####.', '.####.', '.####.', '......'].join('\n'))
    expect(
      picture(
        cells((s) => rectSpans(1, 1, 4, 3, 1, s)),
        6,
        5,
      ),
    ).toBe(['......', '.####.', '.#..#.', '.####.', '......'].join('\n'))
  })

  it('ellipses are symmetric for odd and even sizes', () => {
    for (const [w, h] of [
      [7, 7],
      [8, 8],
      [9, 4],
      [1, 5],
      [2, 2],
    ]) {
      const s = cells((span) => ellipseSpans(0, 0, w - 1, h - 1, span))
      for (const key of s) {
        const [x, y] = key.split(',').map(Number)
        expect(s.has(`${w - 1 - x},${y}`)).toBe(true)
        expect(s.has(`${x},${h - 1 - y}`)).toBe(true)
      }
      // The middle row/column spans the whole box.
      expect(s.has(`0,${Math.floor((h - 1) / 2)}`)).toBe(true)
      expect(s.has(`${Math.floor((w - 1) / 2)},0`)).toBe(true)
    }
  })

  it('a 20 × 20 ellipse is the cylinder of the example scene', () => {
    const scene = JSON.parse(
      readFileSync(
        fileURLToPath(new URL('../../../scenes/cylinder-re100.json', import.meta.url)),
        'utf8',
      ),
    )
    const { layers } = fromScene(scene)
    const cylinder = new Set<string>()
    for (let y = 0; y < layers.height; y++) {
      for (let x = 0; x < layers.width; x++) {
        if (layers.elementId[layers.index(x, y)] === 4) cylinder.add(`${x},${y}`)
      }
    }
    // Centre at cell edge (41, 41), radius 10 cells → cells 31..50 on both axes.
    expect(cells((s) => ellipseSpans(31, 31, 50, 50, s))).toEqual(cylinder)
  })

  it('elliptical outlines are rings of the given thickness', () => {
    const ring = cells((s) => ellipseOutlineSpans(0, 0, 8, 8, 1, s))
    const disk = cells((s) => ellipseSpans(0, 0, 8, 8, s))
    const hole = cells((s) => ellipseSpans(1, 1, 7, 7, s))
    expect(ring.size).toBe(disk.size - hole.size)
    for (const c of hole) expect(ring.has(c)).toBe(false)
    // Too thick for a hole: the whole disk.
    expect(cells((s) => ellipseOutlineSpans(0, 0, 3, 3, 2, s))).toEqual(
      cells((s) => ellipseSpans(0, 0, 3, 3, s)),
    )
  })
})

describe('floodFill', () => {
  /** Fills a picture (top row first) from (x, y), turning '.' into '#'. */
  function fill(rows: string[], x: number, y: number): string {
    const h = rows.length
    const w = rows[0].length
    const grid = rows.map((r) => r.split('')).reverse() // grid[y][x], y up
    floodFill(
      w,
      h,
      x,
      y,
      (cx, cy) => grid[cy][cx] === '.',
      (cy, x0, x1) => {
        for (let cx = x0; cx <= x1; cx++) grid[cy][cx] = '#'
      },
    )
    return grid
      .reverse()
      .map((r) => r.join(''))
      .join('\n')
  }

  it('fills a 4-connected region and stops at walls and diagonals', () => {
    const rows = ['..X...', '.XX.X.', 'X..X..', '..X...']
    expect(fill(rows, 0, 0)).toBe(['..X...', '.XX.X.', 'X##X..', '##X...'].join('\n'))
    expect(fill(rows, 5, 0)).toBe(['..X###', '.XX#X#', 'X..X##', '..X###'].join('\n'))
  })

  it('handles a large empty grid without recursion', () => {
    const n = 1024
    let count = 0
    const seen = new Uint8Array(n * n)
    floodFill(
      n,
      n,
      500,
      500,
      (x, y) => seen[y * n + x] === 0,
      (y, x0, x1) => {
        seen.fill(1, y * n + x0, y * n + x1 + 1)
        count += x1 - x0 + 1
      },
    )
    expect(count).toBe(n * n)
  })

  it('does nothing outside the grid or on a cell that is not inside', () => {
    let calls = 0
    floodFill(
      4,
      4,
      -1,
      0,
      () => true,
      () => calls++,
    )
    floodFill(
      4,
      4,
      0,
      0,
      () => false,
      () => calls++,
    )
    expect(calls).toBe(0)
  })
})
