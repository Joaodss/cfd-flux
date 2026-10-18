// The editor's store (Zustand): the scene table, the view and UI state. Pixel layers are only
// referenced here; tools and the renderer read and write them directly (ADR-005).
import { create } from 'zustand'
import { immer } from 'zustand/middleware/immer'
import { newDocument } from '../scene/defaults.ts'
import type { Document, SceneTable } from '../scene/io.ts'
import { History, type Entry } from './history.ts'
import { CellType, type Layers } from './layers.ts'
import type { Brush } from './tools/raster.ts'
import type { Selection } from './tools/select.ts'
import type { Operation, Paint, ToolId } from './tools/tools.ts'
import { clampZoom, fitView, stepZoom, zoomAt, type View } from './view.ts'

export interface EditorState {
  table: SceneTable
  layers: Layers
  fluidId: Document['fluidId']
  /** Undo/redo of this document (a mutable object: `historyVersion` tells React it changed). */
  history: History
  historyVersion: number
  view: View
  /** Size of the canvas in CSS pixels (set by the canvas component). */
  viewport: { width: number; height: number }
  /** The cell under the pointer, or null when the pointer is off the canvas. */
  cursor: [number, number] | null
  tool: ToolId
  /** What the drawing tools write (the palette chooses it from step 3). */
  paint: Paint
  brush: Brush
  /** Rectangles and ellipses filled, or outlined with the brush width. */
  filled: boolean
  selection: Selection | null
  floating: boolean
  /** Size in cells of the shape being dragged, for the status bar. */
  dragSize: [number, number] | null
  /** Bumped when the layers changed outside the store (tools), so the canvas redraws. */
  layersVersion: number

  load(doc: Document): void
  setViewport(width: number, height: number): void
  setView(view: View): void
  /** Zooms one ladder step in (+1) or out (−1), about a screen point (default: the centre). */
  zoomStep(direction: 1 | -1, at?: [number, number]): void
  zoomTo(zoom: number): void
  fit(): void
  setCursor(cell: [number, number] | null): void
  setTool(tool: ToolId): void
  setPaint(paint: Paint): void
  setBrush(brush: Partial<Brush>): void
  setFilled(filled: boolean): void
  setSelection(selection: Selection | null, floating: boolean): void
  setDragSize(size: [number, number] | null): void
  layersChanged(): void
  /** Records a tool's operation in the history. */
  commit(op: Operation): void
  undo(): void
  redo(): void
}

export const MAX_BRUSH = 64

/** What erasing paints: fluid, or empty cells (gas) in a free-surface scene. */
export function background(table: SceneTable): Paint {
  return {
    cellType: table.physics.freeSurface ? CellType.Empty : CellType.Fluid,
    elementId: 0,
  }
}

export const useEditor = create<EditorState>()(
  immer((set, get) => {
    const initial = newDocument()
    /** Applies an undone or redone entry's table side and tells subscribers. */
    const applied = (e: Entry | null, table: 'before' | 'after') =>
      set((s) => {
        if (!e) return
        if (e.table) s.table = e.table[table]
        s.historyVersion++
        s.layersVersion++
      })
    return {
      table: initial.table,
      layers: initial.layers,
      fluidId: initial.fluidId,
      history: new History(),
      historyVersion: 0,
      view: { zoom: 1, offsetX: 0, offsetY: 0 },
      viewport: { width: 0, height: 0 },
      cursor: null,
      tool: 'pencil',
      paint: { cellType: CellType.Solid, elementId: 0 },
      brush: { size: 1, round: false },
      filled: true,
      selection: null,
      floating: false,
      dragSize: null,
      layersVersion: 0,

      load: (doc) =>
        set((s) => {
          s.table = doc.table
          s.layers = doc.layers
          s.fluidId = doc.fluidId
          s.history = new History()
          s.historyVersion++
          s.cursor = null
          s.selection = null
          s.floating = false
          const { width, height } = s.viewport
          if (width > 0) s.view = fitView(doc.layers.width, doc.layers.height, width, height)
        }),
      setViewport: (width, height) =>
        set((s) => {
          const first = s.viewport.width === 0
          s.viewport = { width, height }
          if (first) s.view = fitView(s.layers.width, s.layers.height, width, height)
        }),
      setView: (view) => set({ view }),
      zoomStep: (direction, at) => {
        const { view, viewport } = get()
        const [sx, sy] = at ?? [viewport.width / 2, viewport.height / 2]
        set({ view: zoomAt(view, stepZoom(view.zoom, direction), sx, sy) })
      },
      zoomTo: (zoom) => {
        const { view, viewport } = get()
        set({ view: zoomAt(view, clampZoom(zoom), viewport.width / 2, viewport.height / 2) })
      },
      fit: () => {
        const { layers, viewport } = get()
        set({ view: fitView(layers.width, layers.height, viewport.width, viewport.height) })
      },
      setCursor: (cell) => {
        const c = get().cursor
        if (c === cell || (c && cell && c[0] === cell[0] && c[1] === cell[1])) return
        set({ cursor: cell })
      },
      setTool: (tool) => set({ tool }),
      setPaint: (paint) => set({ paint }),
      setBrush: (brush) =>
        set((s) => {
          if (brush.size !== undefined) {
            s.brush.size = Math.min(MAX_BRUSH, Math.max(1, Math.round(brush.size)))
          }
          if (brush.round !== undefined) s.brush.round = brush.round
        }),
      setFilled: (filled) => set({ filled }),
      setSelection: (selection, floating) => set({ selection, floating }),
      setDragSize: (size) => {
        const d = get().dragSize
        if (d === size || (d && size && d[0] === size[0] && d[1] === size[1])) return
        set({ dragSize: size })
      },
      layersChanged: () => set((s) => void s.layersVersion++),
      commit: (op) =>
        set((s) => {
          s.history.push({ label: op.label, tiles: op.tiles })
          s.historyVersion++
          s.layersVersion++
        }),
      undo: () => {
        const { history, layers } = get()
        applied(history.undo(layers), 'before')
      },
      redo: () => {
        const { history, layers } = get()
        applied(history.redo(layers), 'after')
      },
    }
  }),
)

// Development builds expose the store for debugging from the console and for browser tests.
if (import.meta.env.DEV) {
  ;(globalThis as { __editor?: typeof useEditor }).__editor = useEditor
}
