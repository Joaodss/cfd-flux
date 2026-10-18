// Bottom status bar: the cell under the pointer, its physical position and contents, zoom, size.
import { useEditor } from '../editor/store.ts'
import { CELL_NAMES, formatLength, formatZoom } from './format.ts'

export function StatusBar() {
  const cursor = useEditor((s) => s.cursor)
  const zoom = useEditor((s) => s.view.zoom)
  const grid = useEditor((s) => s.table.grid)
  const elements = useEditor((s) => s.table.elements)
  // The layers are not React state: read the cell here, re-rendered because `cursor` changed.
  const layers = useEditor.getState().layers

  let where = ''
  let what = ''
  if (cursor && layers.contains(cursor[0], cursor[1])) {
    const [x, y] = cursor
    const i = layers.index(x, y)
    where = `(${x}, ${y}) · ${formatLength((x + 0.5) * grid.cellSize)}, ${formatLength((y + 0.5) * grid.cellSize)}`
    const id = layers.elementId[i]
    const el = id ? elements.find((e) => e.id === id) : undefined
    what = CELL_NAMES[layers.cellType[i]] ?? `cell type ${layers.cellType[i]}`
    if (id) what += ` · ${el?.name ?? `element ${id}`}${el ? '' : ' (missing)'}`
  }

  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t border-border bg-muted px-3 text-xs text-muted-foreground tabular-nums">
      <span className="min-w-56" data-testid="status-cursor">
        {where || '—'}
      </span>
      <span className="min-w-40">{what}</span>
      <span className="ml-auto">{formatZoom(zoom)}</span>
      <span>
        {grid.width} × {grid.height} cells · {formatLength(grid.width * grid.cellSize)} ×{' '}
        {formatLength(grid.height * grid.cellSize)}
      </span>
    </footer>
  )
}
