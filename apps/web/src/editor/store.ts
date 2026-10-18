// The editor's store (Zustand): the scene table, the view and UI state. Pixel layers are only
// referenced here; tools and the renderer read and write them directly (ADR-005).
import type { Draft } from 'immer'
import { create } from 'zustand'
import { immer } from 'zustand/middleware/immer'
import { newDocument } from '../scene/defaults.ts'
import type { Validation } from '../api/client.ts'
import type { Document, SceneTable } from '../scene/io.ts'
import type { ElementKind } from './colours.ts'
import { defaultElement, nextElementId, type PaletteKind } from './elements.ts'
import { History, type Entry } from './history.ts'
import { CellType, type Layers } from './layers.ts'
import type { Brush } from './tools/raster.ts'
import type { Selection } from './tools/select.ts'
import type { Operation, Paint, ToolId } from './tools/tools.ts'
import { resizeLayers, resizeTable, scaleLayers, scaleTable } from './resize.ts'
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
  /** The palette's choice: what the drawing tools paint (probes have their own tool). */
  palette: PaletteKind
  /**
   * The element each kind paints with; null (or an id that no longer exists) means the next
   * stroke of that kind creates a new element.
   */
  activeElement: Record<ElementKind, number | null>
  /** The element shown in the properties panel. */
  selectedElement: number | null
  /** Index in `table.probes` of the selected probe. */
  selectedProbe: number | null
  /** The properties panel's tab. */
  panelTab: PanelTab
  brush: Brush
  /** Rectangles and ellipses filled, or outlined with the brush width. */
  filled: boolean
  selection: Selection | null
  floating: boolean
  /** Size in cells of the shape being dragged, for the status bar. */
  dragSize: [number, number] | null
  /** Bumped when the layers changed outside the store (tools), so the canvas redraws. */
  layersVersion: number
  /** Where the document was opened from or last saved to (where "Save" writes). */
  source: DocSource | null
  /** `historyVersion` at the last load or save: the document is dirty when they differ. */
  savedVersion: number
  /** A message for the bar under the header (saved, errors), or null. */
  notice: Notice | null
  /** The server's verdict on the current document (debounced; see app/validation.ts). */
  validation: ValidationState
  /** A cell to highlight briefly (an issue the user clicked). */
  flash: [number, number] | null

  load(doc: Document, source?: DocSource | null): void
  /** The document was saved to `source`. */
  markSaved(source: DocSource): void
  notify(kind: Notice['kind'], text: string): void
  dismissNotice(): void
  setValidation(v: ValidationState): void
  /** Centres the view on a cell (at least 8 px/cell) and highlights it. */
  showCell(cell: [number, number]): void
  setViewport(width: number, height: number): void
  setView(view: View): void
  /** Zooms one ladder step in (+1) or out (−1), about a screen point (default: the centre). */
  zoomStep(direction: 1 | -1, at?: [number, number]): void
  zoomTo(zoom: number): void
  fit(): void
  setCursor(cell: [number, number] | null): void
  setTool(tool: ToolId): void
  setPalette(kind: PaletteKind): void
  /** Makes `id` the active element of its kind (null: the next stroke creates a new one). */
  setActiveElement(kind: ElementKind, id: number | null): void
  selectElement(id: number | null): void
  selectProbe(index: number | null): void
  setPanelTab(tab: PanelTab): void
  /**
   * Adds a new element of `kind` with the default properties and makes it active and selected;
   * no history entry (the stroke that paints it records the table change). Null when ids run out.
   */
  createElement(kind: ElementKind): number | null
  /** Changes the table without a history entry (live edits during a drag). */
  editTable(recipe: (t: Draft<SceneTable>) => void): void
  /** Puts back a table taken earlier, without a history entry. */
  restoreTable(table: SceneTable): void
  /** Records a table change made since `before` (with `editTable`) as one history entry. */
  commitTable(label: string, before: SceneTable): void
  /** Changes the table as one history entry (a property edit). */
  updateTable(label: string, recipe: (t: Draft<SceneTable>) => void): void
  /** Crops or extends the grid, anchored at the bottom-left cell (one history entry). */
  resizeGrid(width: number, height: number): void
  /** Refines the drawing: each cell becomes k × k cells of 1/k the size (one history entry). */
  scaleDrawing(k: number): void
  setBrush(brush: Partial<Brush>): void
  setFilled(filled: boolean): void
  setSelection(selection: Selection | null, floating: boolean): void
  setDragSize(size: [number, number] | null): void
  layersChanged(): void
  /** Records a tool's operation in the history, with the table change it made, if any. */
  commit(op: Operation & Pick<Entry, 'table'>): void
  undo(): void
  redo(): void
}

export const MAX_BRUSH = 64

export type PanelTab = 'element' | 'scene' | 'run'

/** Where a document lives: a file (with its handle where the browser gives one), the server, an example. */
export type DocSource =
  | { kind: 'file'; name: string; handle?: FileSystemFileHandle }
  | { kind: 'server'; id: string; name: string }
  | { kind: 'example'; id: string }

/** Where validation stands: running, the server's answer, or no server. */
export type ValidationState =
  | { status: 'idle' }
  | { status: 'pending'; last: Validation | null }
  | { status: 'done'; result: Validation }
  | { status: 'offline'; message: string }

export interface Notice {
  kind: 'info' | 'error'
  text: string
}

/** Whether the document has changes that were not saved. */
export function isDirty(s: Pick<EditorState, 'historyVersion' | 'savedVersion'>): boolean {
  return s.historyVersion !== s.savedVersion
}

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
    /** Replaces the grid (layers and table) as one history entry. */
    const replaceGrid = (label: string, table: SceneTable, after: Layers) => {
      const { history, table: before, layers } = get()
      history.push({
        label,
        tiles: new Map(),
        table: { before, after: table },
        layers: { before: layers, after },
      })
      set((s) => {
        s.table = table
        s.layers = after
        s.selectedProbe = null
        s.historyVersion++
        s.layersVersion++
      })
    }
    /** Applies an undone or redone entry's table side and tells subscribers. */
    const applied = (e: Entry | null, table: 'before' | 'after') =>
      set((s) => {
        if (!e) return
        if (e.table) s.table = e.table[table]
        if (e.layers) s.layers = e.layers[table]
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
      palette: 'wall',
      activeElement: { wall: null, inlet: null, outlet: null, heatSource: null },
      selectedElement: null,
      selectedProbe: null,
      panelTab: 'scene',
      brush: { size: 1, round: false },
      filled: true,
      selection: null,
      floating: false,
      dragSize: null,
      layersVersion: 0,
      source: null,
      savedVersion: 0,
      notice: null,
      validation: { status: 'idle' },
      flash: null,
      setValidation: (validation) => set({ validation }),
      showCell: ([x, y]) => {
        const { view, viewport, layers } = get()
        const zoom = Math.max(view.zoom, 8)
        set({
          flash: [x, y],
          view: {
            zoom,
            offsetX: viewport.width / 2 - (x + 0.5) * zoom,
            offsetY: viewport.height / 2 - (layers.height - y - 0.5) * zoom,
          },
        })
        // The highlight fades after two seconds (unless another cell was shown since).
        const shown = get().flash
        setTimeout(() => get().flash === shown && set({ flash: null }), 2000)
      },
      notify: (kind, text) => set({ notice: { kind, text } }),
      dismissNotice: () => set({ notice: null }),

      markSaved: (source) =>
        set((s) => {
          s.source = source
          s.savedVersion = s.historyVersion
        }),

      load: (doc, source = null) =>
        set((s) => {
          s.table = doc.table
          s.layers = doc.layers
          s.fluidId = doc.fluidId
          s.history = new History()
          s.historyVersion++
          s.savedVersion = s.historyVersion
          s.source = source
          s.cursor = null
          s.selection = null
          s.floating = false
          s.activeElement = { wall: null, inlet: null, outlet: null, heatSource: null }
          s.selectedElement = null
          s.selectedProbe = null
          if (s.palette === 'empty' && !doc.table.physics.freeSurface) s.palette = 'fluid'
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
      setPalette: (palette) => set({ palette }),
      setActiveElement: (kind, id) =>
        set((s) => {
          s.activeElement[kind] = id
          if (id !== null) {
            s.selectedElement = id
            s.panelTab = 'element'
          }
        }),
      selectElement: (id) =>
        set((s) => {
          s.selectedElement = id
          if (id !== null) s.panelTab = 'element'
        }),
      selectProbe: (index) => set({ selectedProbe: index }),
      setPanelTab: (panelTab) => set({ panelTab }),
      createElement: (kind) => {
        const { table } = get()
        const id = nextElementId(table.elements)
        if (id === null) return null
        const element = defaultElement(table, kind, id)
        set((s) => {
          s.table.elements.push(element)
          s.activeElement[kind] = id
          s.selectedElement = id
          s.panelTab = 'element'
        })
        return id
      },
      editTable: (recipe) => set((s) => void recipe(s.table)),
      restoreTable: (table) => set({ table }),
      updateTable: (label, recipe) => {
        const before = get().table
        set((s) => void recipe(s.table))
        get().commitTable(label, before)
      },
      resizeGrid: (width, height) => {
        const { table, layers } = get()
        if (width === layers.width && height === layers.height) return
        const after = resizeLayers(layers, width, height, background(table))
        replaceGrid('Resize grid', resizeTable(table, width, height), after)
      },
      scaleDrawing: (k) => {
        const { table, layers } = get()
        replaceGrid(`Scale ×${k}`, scaleTable(table, k), scaleLayers(layers, k))
      },
      commitTable: (label, before) => {
        // Read the table outside a recipe: inside one, `s.table` is an immer draft (a proxy
        // that is revoked when the recipe ends), not the immutable value the history must keep.
        const { table, history } = get()
        if (table === before) return
        history.push({ label, tiles: new Map(), table: { before, after: table } })
        set((s) => void s.historyVersion++)
      },
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
          s.history.push({ label: op.label, tiles: op.tiles, table: op.table })
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
