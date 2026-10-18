// Connects pointer and keyboard input to the tools: one instance of each tool, the drag in
// progress, and the editor commands (undo/redo, clipboard, tool and brush shortcuts).
import { background, MAX_BRUSH, useEditor } from '../store.ts'
import { screenToCell } from '../view.ts'
import { SelectTool } from './select.ts'
import {
  Bucket,
  Freehand,
  Shape,
  type DrawTool,
  type Modifiers,
  type ToolContext,
  type ToolId,
} from './tools.ts'

export const selectTool = new SelectTool((selection, floating) =>
  useEditor.getState().setSelection(selection, floating),
)

const TOOLS: Record<Exclude<ToolId, 'hand'>, DrawTool> = {
  pencil: new Freehand(false),
  eraser: new Freehand(true),
  line: new Shape('line'),
  rect: new Shape('rect'),
  ellipse: new Shape('ellipse'),
  bucket: new Bucket(),
  select: selectTool,
}

/** The tool of the drag in progress (pointer down), if any. */
let active: DrawTool | null = null

function context(): ToolContext {
  const s = useEditor.getState()
  return {
    layers: s.layers,
    paint: s.paint,
    background: background(s.table),
    brush: s.brush,
    filled: s.filled,
    commit: s.commit,
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
    selectTool.reset()
  }
})

/** Starts a drag with the current tool; false for the hand tool (the canvas pans instead). */
export function pointerDown(cell: [number, number], m: Modifiers): boolean {
  const s = useEditor.getState()
  if (s.tool === 'hand') return false
  active = TOOLS[s.tool]
  active.down(cell, m, context())
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
  changed()
}

export function setTool(tool: ToolId): void {
  const s = useEditor.getState()
  if (active) return
  if (s.tool === 'select' && tool !== 'select') {
    selectTool.drop()
    selectTool.cancel() // deselect
    changed()
  }
  s.setTool(tool)
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
    case 'Escape':
      if (active) {
        active.cancel()
        active = null
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
