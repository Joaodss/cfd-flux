// Right panel: the palette, then a summary of the scene. The property editors join it in step 4.
import { useEditor } from '../editor/store.ts'
import { formatLength } from './format.ts'
import { Palette } from './Palette.tsx'

function SceneSummary() {
  const table = useEditor((s) => s.table)
  const { grid } = table
  return (
    <section>
      <h3 className="mb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        Scene
      </h3>
      <p className="font-medium">{table.name}</p>
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
  )
}

export function ScenePanel() {
  return (
    <aside className="flex w-72 shrink-0 flex-col gap-5 overflow-y-auto border-l border-border p-3 text-sm">
      <Palette />
      <SceneSummary />
    </aside>
  )
}
