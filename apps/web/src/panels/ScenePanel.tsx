// Right panel (step 1): a read-only summary of the scene and its elements. The palette and the
// property editors replace it in steps 3–4.
import { coloursFor, rgbaCss } from '../editor/colours.ts'
import { useEditor } from '../editor/store.ts'
import { formatLength } from './format.ts'

const KIND_NAMES = { wall: 'Wall', inlet: 'Inlet', outlet: 'Outlet', heatSource: 'Heat source' }

export function ScenePanel() {
  const table = useEditor((s) => s.table)
  const colours = coloursFor(table.elements)
  const { grid } = table
  return (
    <aside className="flex w-72 shrink-0 flex-col gap-4 overflow-y-auto border-l border-border p-3 text-sm">
      <section>
        <h2 className="mb-1 font-medium">{table.name}</h2>
        {table.description && <p className="text-xs text-muted-foreground">{table.description}</p>}
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
          <dt className="text-muted-foreground">Grid</dt>
          <dd>
            {grid.width} × {grid.height} cells of {formatLength(grid.cellSize)}
          </dd>
          <dt className="text-muted-foreground">Size</dt>
          <dd>
            {formatLength(grid.width * grid.cellSize)} × {formatLength(grid.height * grid.cellSize)}
          </dd>
          <dt className="text-muted-foreground">Fluid</dt>
          <dd>{table.fluids.map((f) => f.name).join(', ')}</dd>
        </dl>
      </section>
      <section>
        <h3 className="mb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Elements
        </h3>
        {table.elements.length === 0 ? (
          <p className="text-xs text-muted-foreground">None yet.</p>
        ) : (
          <ul className="flex flex-col gap-1" data-testid="element-list">
            {table.elements.map((e) => (
              <li key={e.id} className="flex items-center gap-2">
                <span
                  className="size-3 shrink-0 rounded-sm border border-border"
                  style={{ background: rgbaCss(colours.elements.get(e.id)!) }}
                />
                <span className="truncate">{e.name ?? `${KIND_NAMES[e.kind]} ${e.id}`}</span>
                <span className="ml-auto text-xs text-muted-foreground">{KIND_NAMES[e.kind]}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </aside>
  )
}
