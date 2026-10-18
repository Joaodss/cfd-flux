// The palette: what the drawing tools paint. Each element kind lists its elements (colour, cell
// count); clicking one makes it the active element, "+" makes the next stroke create a new one.
import { Plus, Trash2 } from 'lucide-react'
import { CELL_COLOURS, coloursFor, rgbaCss, type ElementKind } from '../editor/colours.ts'
import {
  cachedCellCounts,
  ELEMENT_KINDS,
  elementLabel,
  KIND_NAMES,
  validElement,
  type PaletteKind,
} from '../editor/elements.ts'
import { CellType } from '../editor/layers.ts'
import { useEditor } from '../editor/store.ts'
import { choosePalette, deleteElement, newElement, probeTool } from '../editor/tools/controller.ts'
import { cn } from '../lib/utils.ts'

const KIND_SWATCH: Record<PaletteKind, string> = {
  fluid: rgbaCss(CELL_COLOURS[CellType.Fluid]),
  wall: rgbaCss(CELL_COLOURS[CellType.Solid]),
  inlet: rgbaCss(CELL_COLOURS[CellType.Inlet]),
  outlet: rgbaCss(CELL_COLOURS[CellType.Outlet]),
  heatSource: rgbaCss(CELL_COLOURS[CellType.HeatSource]),
  empty: rgbaCss(CELL_COLOURS[CellType.Empty]),
  probe: '#a855f7',
}

const SHORTCUTS: Partial<Record<PaletteKind, string>> = { probe: 'P' }

function KindRow(props: { kind: PaletteKind; active: boolean; onNew?: () => void }) {
  return (
    <div className="flex items-center">
      <button
        type="button"
        data-testid={`palette-${props.kind}`}
        aria-pressed={props.active}
        onClick={() => choosePalette(props.kind)}
        className={cn(
          'flex h-7 flex-1 items-center gap-2 rounded-md px-2 text-left text-sm',
          props.active ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60',
        )}
      >
        <span
          className={cn(
            'size-3.5 shrink-0 border border-border',
            props.kind === 'probe' ? 'rounded-full' : 'rounded-sm',
          )}
          style={{ background: KIND_SWATCH[props.kind] }}
        />
        {KIND_NAMES[props.kind]}
        {SHORTCUTS[props.kind] && (
          <span className="ml-auto text-xs text-muted-foreground">{SHORTCUTS[props.kind]}</span>
        )}
      </button>
      {props.onNew && (
        <button
          type="button"
          title={`New ${KIND_NAMES[props.kind].toLowerCase()} element (N): the next stroke creates it`}
          aria-label={`New ${KIND_NAMES[props.kind]}`}
          data-testid={`new-${props.kind}`}
          onClick={props.onNew}
          className="ml-1 grid size-7 place-items-center rounded-md text-muted-foreground hover:bg-accent [&_svg]:size-4"
        >
          <Plus />
        </button>
      )}
    </div>
  )
}

export function Palette() {
  const palette = useEditor((s) => s.palette)
  const elements = useEditor((s) => s.table.elements)
  const probes = useEditor((s) => s.table.probes)
  const freeSurface = useEditor((s) => s.table.physics.freeSurface)
  const activeElement = useEditor((s) => s.activeElement)
  const selectedElement = useEditor((s) => s.selectedElement)
  const selectedProbe = useEditor((s) => s.selectedProbe)
  const layers = useEditor((s) => s.layers)
  const historyVersion = useEditor((s) => s.historyVersion)
  const colours = coloursFor(elements)
  const counts = cachedCellCounts(layers, historyVersion)

  const choose = (kind: ElementKind, id: number) => {
    choosePalette(kind)
    useEditor.getState().setActiveElement(kind, id)
  }

  return (
    <section className="flex flex-col gap-0.5" data-testid="palette">
      <h3 className="mb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
        Palette
      </h3>
      <KindRow kind="fluid" active={palette === 'fluid'} />
      {ELEMENT_KINDS.map((kind) => {
        const ofKind = elements.filter((e) => e.kind === kind)
        const active = validElement(elements, kind, activeElement[kind])
        return (
          <div key={kind}>
            <KindRow
              kind={kind}
              active={palette === kind}
              onNew={() => {
                choosePalette(kind)
                newElement()
              }}
            />
            <ul className="ml-5 flex flex-col">
              {ofKind.map((e) => (
                <li key={e.id} className="group flex items-center">
                  <button
                    type="button"
                    data-testid={`element-${e.id}`}
                    onClick={() => choose(kind, e.id)}
                    className={cn(
                      'flex h-6 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-xs',
                      active === e.id && palette === kind && 'ring-1 ring-primary',
                      selectedElement === e.id ? 'bg-accent' : 'hover:bg-accent/60',
                    )}
                  >
                    <span
                      className="size-3 shrink-0 rounded-sm border border-border"
                      style={{ background: rgbaCss(colours.elements.get(e.id)!) }}
                    />
                    <span className="truncate">{elementLabel(e)}</span>
                    <span className="ml-auto text-muted-foreground tabular-nums">
                      {counts[e.id] === 0 ? '0 cells' : counts[e.id].toLocaleString('en')}
                    </span>
                  </button>
                  <button
                    type="button"
                    title={`Delete ${elementLabel(e)} (its cells become ${freeSurface ? 'empty' : 'fluid'})`}
                    aria-label={`Delete ${elementLabel(e)}`}
                    onClick={() => deleteElement(e.id)}
                    className="invisible ml-1 grid size-6 place-items-center rounded-md text-muted-foreground group-hover:visible hover:bg-accent hover:text-red-400 [&_svg]:size-3.5"
                  >
                    <Trash2 />
                  </button>
                </li>
              ))}
              {palette === kind && active === null && (
                <li className="px-2 py-0.5 text-xs text-muted-foreground italic">
                  New {KIND_NAMES[kind].toLowerCase()} on the next stroke
                </li>
              )}
            </ul>
          </div>
        )
      })}
      {freeSurface && <KindRow kind="empty" active={palette === 'empty'} />}
      <KindRow kind="probe" active={palette === 'probe'} />
      <ul className="ml-5 flex flex-col">
        {(probes ?? []).map((p, i) => (
          <li key={i} className="group flex items-center">
            <button
              type="button"
              onClick={() => {
                choosePalette('probe')
                useEditor.getState().selectProbe(i)
              }}
              className={cn(
                'flex h-6 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-xs',
                selectedProbe === i ? 'bg-accent' : 'hover:bg-accent/60',
              )}
            >
              <span className="truncate">{p.name}</span>
              <span className="ml-auto text-muted-foreground tabular-nums">
                ({p.position[0]}, {p.position[1]})
              </span>
            </button>
            <button
              type="button"
              aria-label={`Delete ${p.name}`}
              title={`Delete ${p.name}`}
              onClick={() => {
                useEditor.getState().selectProbe(i)
                probeTool.deleteSelected()
              }}
              className="invisible ml-1 grid size-6 place-items-center rounded-md text-muted-foreground group-hover:visible hover:bg-accent hover:text-red-400 [&_svg]:size-3.5"
            >
              <Trash2 />
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}
