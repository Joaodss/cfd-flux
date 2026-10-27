// Connects pointer and keyboard input to the tools: one instance of each tool, the drag in
// progress, the palette's paint (creating elements when needed), and the editor commands
// (undo/redo, clipboard, element and probe commands, shortcuts).
import type { SceneTable } from '../../scene/io.ts'
import { KIND_CELL } from '../colours.ts'
import { INWARD, paintEdge, SIDE_NAMES, WALL_EDGES, type Side } from '../edges.ts'
import { elementLabel, isElementKind, plainPaint, validElement } from '../elements.ts'
import { Stroke } from '../history.ts'
import { CellType } from '../layers.ts'
import { background, MAX_BRUSH, useEditor } from '../store.ts'
import { screenToCell } from '../view.ts'
import { ProbeTool } from './probe.ts'
import { SelectTool } from './select.ts'
import {
  Bucket,
  Freehand,
  Shape,
  type DrawTool,
  type Modifiers,
  type Operation,
  type Paint,
  type ToolContext,
  type ToolId,
} from './tools.ts'

export const selectTool = new SelectTool((selection, floating) =>
  useEditor.getState().setSelection(selection, floating),
)
export const probeTool = new ProbeTool()

const TOOLS: Record<Exclude<ToolId, 'hand' | 'picker'>, DrawTool> = {
  pencil: new Freehand(false),
  eraser: new Freehand(true),
  line: new Shape('line'),
  rect: new Shape('rect'),
  ellipse: new Shape('ellipse'),
  bucket: new Bucket(),
  select: selectTool,
  probe: probeTool,
}

/** Tools that write the palette's paint (the others write the background or nothing). */
const PAINTING: ReadonlySet<ToolId> = new Set(['pencil', 'line', 'rect', 'ellipse', 'bucket'])

/** The tool of the drag in progress (pointer down), if any. */
let active: DrawTool | null = null
/**
 * The table before an element was created for the drag in progress: the creation and the
 * stroke become one history entry, and a stroke that changes nothing removes the element again.
 */
let tableBeforeCreate: SceneTable | null = null

/** Records an operation, with the element creation that preceded it. */
function commit(op: Operation): void {
  const s = useEditor.getState()
  const table = tableBeforeCreate ? { before: tableBeforeCreate, after: s.table } : undefined
  tableBeforeCreate = null
  s.commit({ ...op, table })
}

/**
 * The paint of the palette's current kind. For an element kind without a (still existing) active
 * element, `create` adds one; otherwise null is returned.
 */
function palettePaint(create: boolean): Paint | null {
  const s = useEditor.getState()
  const kind = s.palette
  const plain = plainPaint(kind)
  if (plain || !isElementKind(kind)) return plain
  let id = validElement(s.table.elements, kind, s.activeElement[kind])
  if (id === null && create) {
    const before = s.table
    id = s.createElement(kind)
    if (id !== null) tableBeforeCreate = before
  }
  return id === null ? null : { cellType: KIND_CELL[kind], elementId: id }
}

function context(paint: Paint | null = null): ToolContext {
  const s = useEditor.getState()
  const bg = background(s.table)
  return {
    layers: s.layers,
    paint: paint ?? bg,
    background: bg,
    brush: s.brush,
    filled: s.filled,
    commit,
  }
}

/** After any change to the cells: redraw, and show the drag's size. */
function changed(): void {
  const s = useEditor.getState()
  s.layersChanged()
  s.setDragSize(active?.size ?? null)
}

// A new document: forget the drag and the selection (they belong to the old layers).
let layers = useEditor.getState().layers
useEditor.subscribe((s) => {
  if (s.layers !== layers) {
    layers = s.layers
    active = null
    tableBeforeCreate = null
    selectTool.reset()
  }
})

/** Eyedropper: makes the clicked cell's kind (and element) the palette's paint. */
export function pick([x, y]: [number, number]): void {
  const s = useEditor.getState()
  if (!s.layers.contains(x, y)) return
  const i = s.layers.index(x, y)
  const id = s.layers.elementId[i]
  const ct = s.layers.cellType[i]
  const element = id ? s.table.elements.find((e) => e.id === id) : undefined
  if (element) {
    s.setPalette(element.kind)
    s.setActiveElement(element.kind, element.id)
  } else if (ct === CellType.Empty) {
    s.setPalette('empty')
  } else if (ct === CellType.Fluid) {
    s.setPalette('fluid')
  }
}

/** Starts a drag with the current tool; false for the hand tool (the canvas pans instead). */
export function pointerDown(cell: [number, number], m: Modifiers): boolean {
  const s = useEditor.getState()
  const tool = s.tool
  if (tool === 'hand') return false
  // The eyedropper, or Alt+click with a tool that has no other use for Alt.
  if (
    tool === 'picker' ||
    (m.alt && (tool === 'pencil' || tool === 'eraser' || tool === 'bucket'))
  ) {
    pick(cell)
    return true
  }
  const paint = PAINTING.has(tool) ? palettePaint(true) : null
  if (PAINTING.has(tool) && !paint) return true // out of element ids: nothing to paint with
  active = TOOLS[tool]
  active.down(cell, m, context(paint))
  changed()
  return true
}

export function pointerMove(cell: [number, number], m: Modifiers): void {
  if (!active) return
  active.move(cell, m)
  changed()
}

export function pointerUp(cell: [number, number], m: Modifiers): void {
  if (!active) return
  active.up(cell, m)
  active = null
  dropUnusedElement()
  changed()
}

/** A stroke that changed nothing: remove the element created for it. */
function dropUnusedElement(): void {
  if (!tableBeforeCreate) return
  useEditor.getState().restoreTable(tableBeforeCreate)
  tableBeforeCreate = null
}

export function setTool(tool: ToolId): void {
  const s = useEditor.getState()
  if (active) return
  if (s.tool === 'select' && tool !== 'select') {
    selectTool.drop()
    selectTool.cancel() // deselect
    changed()
  }
  if (tool === 'probe') s.setPalette('probe')
  else if (s.palette === 'probe' && tool !== 'hand' && tool !== 'picker') s.setPalette('wall')
  s.setTool(tool)
}

/** Chooses a palette kind; probes switch to the probe tool, other kinds away from it. */
export function choosePalette(kind: ReturnType<typeof useEditor.getState>['palette']): void {
  const s = useEditor.getState()
  if (kind === 'probe') {
    setTool('probe')
    return
  }
  s.setPalette(kind)
  if (s.tool === 'probe' || s.tool === 'eraser' || s.tool === 'select' || s.tool === 'picker') {
    setTool('pencil')
  }
}

/** "New element": the next stroke of the palette's kind creates another element. */
export function newElement(): void {
  const s = useEditor.getState()
  if (isElementKind(s.palette)) s.setActiveElement(s.palette, null)
}

/** Deletes an element: its cells become background, and it leaves the table (one undo entry). */
export function deleteElement(id: number): void {
  if (active) return
  selectTool.drop()
  const s = useEditor.getState()
  const element = s.table.elements.find((e) => e.id === id)
  if (!element) return
  const l = s.layers
  const bg = background(s.table)
  const stroke = new Stroke(l)
  for (let y = 0; y < l.height; y++) {
    const row = y * l.width
    for (let x = 0; x < l.width; x++) {
      if (l.elementId[row + x] === id) stroke.write(x, y, bg.cellType as CellType, bg.elementId)
    }
  }
  const before = s.table
  s.editTable((t) => {
    t.elements = t.elements.filter((e) => e.id !== id)
  })
  if (s.selectedElement === id) s.selectElement(null)
  tableBeforeCreate = before // reuse: the table change rides on the stroke's entry
  const label = `Delete ${elementLabel(element)}`
  const tiles = stroke.finish()
  if (tiles.size > 0) commit({ label, tiles })
  else {
    tableBeforeCreate = null
    useEditor.getState().commitTable(label, before)
  }
  changed()
}

/**
 * "Make inlet" / "Make outlet" on a domain edge: a new element of `kind` (an inlet blowing into
 * the domain) painted on the edge's cells, except drawn walls and heat sources — the same cells as
 * a drag along the edge between walls. The creation and the cells are one undo entry; the new
 * element is selected, and the Scene tab stays open (for the next edge).
 */
export function makeEdge(side: Side, kind: 'inlet' | 'outlet'): void {
  if (active) return
  selectTool.drop()
  const s = useEditor.getState()
  if ((s.table.grid.edges ?? WALL_EDGES)[side] === 'periodic') return
  const before = s.table
  const tab = s.panelTab
  const id = s.createElement(kind)
  if (id === null) return
  s.editTable((t) => {
    const e = t.elements.find((x) => x.id === id)
    if (e?.kind === 'inlet' && e.velocity.type === 'uniform') {
      const speed = Math.hypot(...e.velocity.value)
      const [nx, ny] = INWARD[side]
      e.velocity.value = [nx * speed, ny * speed]
    }
  })
  s.setPanelTab(tab)
  const stroke = new Stroke(s.layers)
  paintEdge(stroke, side, { cellType: KIND_CELL[kind], elementId: id })
  tableBeforeCreate = before
  const tiles = stroke.finish()
  if (tiles.size > 0) commit({ label: `${SIDE_NAMES[side]} edge: ${kind}`, tiles })
  else dropUnusedElement() // every edge cell is a drawn wall: nothing to open
  changed()
}

/**
 * Places an imported mask as a floating selection painted with the palette's paint, centred in
 * the view; the select tool then moves it, and Enter (or a click outside) drops it. An element
 * created for it is recorded first, as its own undo entry. Returns an error message, or null.
 */
export function floatMask(mask: {
  width: number
  height: number
  cells: Uint8Array
}): string | null {
  if (active) return 'finish the current drag first'
  const s = useEditor.getState()
  if (s.palette === 'probe')
    return 'choose a cell type or element in the palette to paint the image with'
  const before = s.table
  const paint = palettePaint(true)
  if (!paint) return 'no element ids left'
  if (tableBeforeCreate) {
    tableBeforeCreate = null
    useEditor.getState().commitTable('New element for the image', before)
  }
  const n = mask.width * mask.height
  const block = {
    width: mask.width,
    height: mask.height,
    cellType: new Uint8Array(n).fill(paint.cellType),
    elementId: new Uint16Array(n).fill(paint.elementId),
    mask: mask.cells,
  }
  const { view, viewport, layers } = useEditor.getState()
  const [cx, cy] = screenToCell(view, layers.height, viewport.width / 2, viewport.height / 2)
  // Centred in the view, but kept inside the grid when it fits (the view may show the margin).
  const place = (c: number, size: number, grid: number) =>
    Math.max(0, Math.min(c - Math.floor(size / 2), grid - size))
  setTool('select')
  selectTool.float(
    block,
    [place(cx, mask.width, layers.width), place(cy, mask.height, layers.height)],
    'Import image',
    context(),
  )
  changed()
  return null
}

export function undo(): void {
  if (active) return
  // Undoing while a selection floats puts it back where it came from.
  if (selectTool.isFloating) selectTool.cancel()
  else useEditor.getState().undo()
  changed()
}

export function redo(): void {
  if (active) return
  selectTool.drop()
  useEditor.getState().redo()
  changed()
}

/** Where a paste lands: the cell under the pointer, or the centre of the view. */
function pasteTarget(): [number, number] {
  const s = useEditor.getState()
  if (s.cursor && s.layers.contains(s.cursor[0], s.cursor[1])) return s.cursor
  const { width, height } = s.viewport
  const c = screenToCell(s.view, s.layers.height, width / 2, height / 2)
  return [
    Math.min(Math.max(c[0], 0), s.layers.width - 1),
    Math.min(Math.max(c[1], 0), s.layers.height - 1),
  ]
}

const TOOL_KEYS: Record<string, ToolId> = {
  h: 'hand',
  b: 'pencil',
  e: 'eraser',
  l: 'line',
  r: 'rect',
  o: 'ellipse',
  g: 'bucket',
  m: 'select',
  i: 'picker',
  p: 'probe',
}

/**
 * Handles the editor's keyboard shortcuts; returns whether the key was used (the caller then
 * prevents the browser's default action).
 */
export function handleKey(e: KeyboardEvent): boolean {
  const s = useEditor.getState()
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key
  const mod = e.ctrlKey || e.metaKey
  if (mod) {
    switch (key) {
      case 'z':
        if (e.shiftKey) redo()
        else undo()
        return true
      case 'y':
        redo()
        return true
      case 'a':
        setTool('select')
        selectTool.selectAll(context())
        changed()
        return true
      case 'c':
        return selectTool.copy(context())
      case 'x':
        selectTool.cut(context())
        changed()
        return true
      case 'v':
        setTool('select')
        selectTool.paste(pasteTarget(), context())
        changed()
        return true
    }
    return false
  }
  if (e.altKey) return false
  if (TOOL_KEYS[key]) {
    setTool(TOOL_KEYS[key])
    return true
  }
  const selecting = s.tool === 'select' && s.selection !== null
  const step = e.shiftKey ? 10 : 1
  switch (key) {
    case 'n':
      newElement()
      return true
    case 'Escape':
      if (active) {
        active.cancel()
        active = null
        dropUnusedElement()
      } else if (s.tool === 'select') {
        selectTool.cancel()
      }
      changed()
      return true
    case 'Enter':
      selectTool.drop()
      changed()
      return true
    case 'Delete':
    case 'Backspace':
      if (s.tool === 'probe' && s.selectedProbe !== null) {
        probeTool.deleteSelected()
        changed()
        return true
      }
      if (!selecting) return false
      selectTool.clear(context())
      changed()
      return true
    case 'ArrowLeft':
    case 'ArrowRight':
    case 'ArrowUp':
    case 'ArrowDown': {
      if (!selecting || active) return false
      const dx = key === 'ArrowLeft' ? -step : key === 'ArrowRight' ? step : 0
      const dy = key === 'ArrowDown' ? -step : key === 'ArrowUp' ? step : 0
      selectTool.nudge(dx, dy, context())
      changed()
      return true
    }
    case '[':
      s.setBrush({ size: s.brush.size - 1 })
      return true
    case ']':
      s.setBrush({ size: Math.min(MAX_BRUSH, s.brush.size + 1) })
      return true
    case 'f':
      s.fit()
      return true
    case '+':
    case '=':
      s.zoomStep(1)
      return true
    case '-':
      s.zoomStep(-1)
      return true
    case '1':
      s.zoomTo(1)
      return true
  }
  return false
}
