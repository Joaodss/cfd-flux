// The Scene tab: name, grid (size, cell size; resize and scale), domain edges (their kind, and
// "Make inlet/outlet"), fluids and solid materials (from the preset library or custom), initial
// conditions, physics and the reference scales of reported numbers. Every edit is
// one history entry; values stay in SI units (the solver converts to lattice units).
import { Plus, Trash2 } from 'lucide-react'
import { useMemo } from 'react'
import {
  CheckField,
  Group,
  NumberField,
  Row,
  SelectField,
  TextField,
  VectorField,
} from '../components/fields.tsx'
import { Button } from '../components/ui/button.tsx'
import {
  edgeSummary,
  SIDE_NAMES,
  SIDES,
  WALL_EDGES,
  type EdgeSummary,
  type Side,
} from '../editor/edges.ts'
import { elementLabel } from '../editor/elements.ts'
import { MAX_GRID_SIDE } from '../editor/resize.ts'
import { useEditor } from '../editor/store.ts'
import { makeEdge } from '../editor/tools/controller.ts'
import type {
  DomainEdges,
  EdgeKind,
  Fluid,
  ReferenceScales,
  SolidMaterial,
} from '../generated/scene.ts'
import { referenceOrNull } from '../scene/io.ts'
import { FLUID_PRESETS, nextId, presetFluid, presetSolid, SOLID_PRESETS } from '../scene/library.ts'
import { currentValidation } from '../app/issues.ts'
import { formatLength, formatValue } from './format.ts'

const update = (
  label: string,
  recipe: Parameters<ReturnType<typeof useEditor.getState>['updateTable']>[1],
) => useEditor.getState().updateTable(label, recipe)

const EDGES: readonly (readonly [EdgeKind, string])[] = [
  ['wall', 'Wall (no slip)'],
  ['periodic', 'Periodic'],
]
const OPPOSITE: Record<Side, Side> = { left: 'right', right: 'left', bottom: 'top', top: 'bottom' }

function GridGroup() {
  const grid = useEditor((s) => s.table.grid)
  const { resizeGrid, scaleDrawing } = useEditor.getState()
  const side = { integer: true, min: 1, max: MAX_GRID_SIDE }
  return (
    <Group title="Grid">
      <NumberField
        label="Width"
        unit="cells"
        value={grid.width}
        rules={side}
        testId="grid-width"
        title="Cropped or extended at the right; the drawing stays anchored at the bottom-left"
        onCommit={(w) => resizeGrid(w, grid.height)}
      />
      <NumberField
        label="Height"
        unit="cells"
        value={grid.height}
        rules={side}
        testId="grid-height"
        title="Cropped or extended at the top; the drawing stays anchored at the bottom-left"
        onCommit={(h) => resizeGrid(grid.width, h)}
      />
      <NumberField
        label="Cell size"
        unit="m"
        value={grid.cellSize}
        rules={{ positive: true }}
        testId="grid-cell-size"
        onCommit={(v) =>
          update('Cell size', (t) => {
            t.grid.cellSize = v
          })
        }
      />
      <Row label="Domain">
        <span className="text-xs tabular-nums">
          {formatLength(grid.width * grid.cellSize)} × {formatLength(grid.height * grid.cellSize)}
        </span>
      </Row>
      <Row label="Refine drawing" title="Each cell becomes k × k cells of 1/k the size">
        {[2, 3, 4].map((k) => (
          <Button
            key={k}
            variant="outline"
            size="sm"
            disabled={Math.max(grid.width, grid.height) * k > MAX_GRID_SIDE}
            onClick={() => scaleDrawing(k)}
          >
            ×{k}
          </Button>
        ))}
      </Row>
    </Group>
  )
}

/** One edge: its kind where nothing is drawn, what is drawn on it, and the inlet/outlet buttons. */
function EdgeRows(props: { side: Side; edges: DomainEdges; summary: EdgeSummary }) {
  const { side, edges, summary } = props
  const elements = useEditor((s) => s.table.elements)
  const name = (id: number) => {
    const e = elements.find((x) => x.id === id)
    return e ? elementLabel(e) : `Element ${id}`
  }
  // What is on the edge, longest first: drawn inlets and outlets by name, walls and periodic once.
  const parts = new Map<string, { length: number; elementId: number }>()
  for (const r of summary.runs) {
    const open = r.kind === 'inlet' || r.kind === 'outlet'
    const key = open ? `e${r.elementId}` : r.kind
    const p = parts.get(key) ?? { length: 0, elementId: open ? r.elementId : 0 }
    p.length += r.end - r.start
    parts.set(key, p)
  }
  const shown = [...parts.entries()].sort((a, b) => b[1].length - a[1].length)
  const why = (kind: 'inlet' | 'outlet'): string | null => {
    if (edges[side] === 'periodic') return 'A periodic edge: make it a wall first'
    if (summary.openable === 0) return 'Every cell of this edge is a drawn wall'
    if (summary.sole?.kind === kind) return `Already an ${kind}: ${name(summary.sole.elementId)}`
    return null
  }
  return (
    <div className="flex flex-col gap-1" data-testid={`edge-${side}`}>
      <SelectField
        label={`${SIDE_NAMES[side]} edge`}
        value={edges[side]}
        options={EDGES}
        testId={`edge-${side}-kind`}
        title="Where nothing is drawn on the edge. Periodic edges come in opposite pairs."
        onChange={(v) =>
          update(`${SIDE_NAMES[side]} edge: ${v}`, (t) => {
            const e = (t.grid.edges ??= { ...edges })
            e[side] = v
            e[OPPOSITE[side]] = v // periodic needs its opposite edge
          })
        }
      />
      <div className="grid grid-cols-[7.5rem_1fr] items-center gap-2">
        <span
          className="truncate text-[11px] text-muted-foreground"
          data-testid={`edge-${side}-summary`}
          title={shown
            .map(([key, p]) => `${p.elementId ? name(p.elementId) : key} (${p.length} cells)`)
            .join(' + ')}
        >
          {shown.map(([key, p], i) => (
            <span key={key}>
              {i > 0 && ' + '}
              {p.elementId ? (
                <button
                  type="button"
                  className="underline decoration-dotted hover:text-foreground"
                  data-testid={`edge-${side}-element-${p.elementId}`}
                  title={`Open ${name(p.elementId)} in the Element tab`}
                  onClick={() => useEditor.getState().selectElement(p.elementId)}
                >
                  {name(p.elementId)}
                </button>
              ) : (
                key
              )}
            </span>
          ))}
        </span>
        <div className="flex gap-1">
          {(['inlet', 'outlet'] as const).map((kind) => (
            <Button
              key={kind}
              variant="outline"
              size="sm"
              disabled={why(kind) !== null}
              title={why(kind) ?? `Paint the ${side} edge as a new ${kind} (drawn walls stay)`}
              data-testid={`make-${kind}-${side}`}
              onClick={() => makeEdge(side, kind)}
            >
              Make {kind}
            </Button>
          ))}
        </div>
      </div>
    </div>
  )
}

/**
 * The domain edges: walls (or periodic) where nothing is drawn — shown on the canvas too — and one
 * click to turn an edge into an inlet or outlet.
 */
function EdgesGroup() {
  const edges = useEditor((s) => s.table.grid.edges) ?? WALL_EDGES
  const layers = useEditor((s) => s.layers)
  // Cells change with history entries (and new grids), not on the moves of a drag.
  const version = useEditor((s) => s.historyVersion)
  const summaries = useMemo(
    () => SIDES.map((side) => edgeSummary(layers, edges, side)),
    // `version` stands for the cells of `layers`, which change in place.
    // oxlint-disable-next-line react-hooks/exhaustive-deps
    [layers, edges, version],
  )
  return (
    <Group title="Domain edges">
      <p className="text-[11px] text-muted-foreground">
        Undrawn edge cells are walls (or periodic). An inlet or outlet replaces the edge's open
        cells; walls drawn on it stay.
      </p>
      {SIDES.map((side, i) => (
        <EdgeRows key={side} side={side} edges={edges} summary={summaries[i]} />
      ))}
    </Group>
  )
}

function FluidEditor(props: { fluid: Fluid; removable: string | null; thermal: boolean }) {
  const f = props.fluid
  const set = (label: string, recipe: (f: Fluid) => void, keepPreset = false) =>
    update(label, (t) => {
      const x = t.fluids.find((y) => y.id === f.id)
      if (!x) return
      recipe(x)
      if (!keepPreset) x.preset = null // edited values are no longer the preset's
    })
  const presetOptions = [
    ['', 'Custom'],
    ...FLUID_PRESETS.map((p) => [p.key, p.name] as const),
  ] as const
  return (
    <div className="flex flex-col gap-1.5 rounded-md border border-border p-2">
      <div className="flex items-center gap-1">
        <span className="text-xs font-medium">Fluid {f.id}</span>
        <button
          type="button"
          className="ml-auto grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-400 disabled:opacity-40 [&_svg]:size-3.5"
          disabled={props.removable !== null}
          title={props.removable ?? `Delete ${f.name}`}
          aria-label={`Delete ${f.name}`}
          onClick={() =>
            update(`Delete ${f.name}`, (t) => {
              t.fluids = t.fluids.filter((x) => x.id !== f.id)
            })
          }
        >
          <Trash2 />
        </button>
      </div>
      <TextField
        label="Name"
        value={f.name}
        onCommit={(v) => set('Fluid name', (x) => void (x.name = v), true)}
      />
      <SelectField
        label="Preset"
        value={f.preset ?? ''}
        options={presetOptions}
        testId={`fluid-${f.id}-preset`}
        onChange={(key) => {
          const p = FLUID_PRESETS.find((x) => x.key === key)
          if (p) set(`Fluid: ${p.name}`, (x) => Object.assign(x, presetFluid(p, f.id)), true)
          else set('Fluid: custom', () => {})
        }}
      />
      <NumberField
        label="Density"
        unit="kg/m³"
        value={f.density}
        rules={{ positive: true }}
        onCommit={(v) => set('Density', (x) => void (x.density = v))}
      />
      <NumberField
        label="Kinematic viscosity"
        unit="m²/s"
        value={f.kinematicViscosity}
        rules={{ positive: true }}
        testId={`fluid-${f.id}-viscosity`}
        onCommit={(v) => set('Viscosity', (x) => void (x.kinematicViscosity = v))}
      />
      {props.thermal && (
        <>
          <NumberField
            label="Conductivity"
            unit="W/(m·K)"
            value={f.thermalConductivity ?? 0}
            rules={{ positive: true }}
            onCommit={(v) => set('Conductivity', (x) => void (x.thermalConductivity = v))}
          />
          <NumberField
            label="Specific heat"
            unit="J/(kg·K)"
            value={f.specificHeat ?? 0}
            rules={{ positive: true }}
            onCommit={(v) => set('Specific heat', (x) => void (x.specificHeat = v))}
          />
          <NumberField
            label="Expansion β"
            unit="1/K"
            value={f.thermalExpansion ?? 0}
            rules={{ min: 0 }}
            onCommit={(v) => set('Expansion coefficient', (x) => void (x.thermalExpansion = v))}
          />
        </>
      )}
    </div>
  )
}

function SolidEditor(props: { material: SolidMaterial }) {
  const m = props.material
  const set = (label: string, recipe: (m: SolidMaterial) => void) =>
    update(label, (t) => {
      const x = (t.solidMaterials ?? []).find((y) => y.id === m.id)
      if (x) recipe(x)
    })
  return (
    <div className="flex flex-col gap-1.5 rounded-md border border-border p-2">
      <div className="flex items-center gap-1">
        <span className="text-xs font-medium">Material {m.id}</span>
        <button
          type="button"
          className="ml-auto grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-red-400 [&_svg]:size-3.5"
          title={`Delete ${m.name} (elements using it get none)`}
          aria-label={`Delete ${m.name}`}
          onClick={() =>
            update(`Delete ${m.name}`, (t) => {
              t.solidMaterials = (t.solidMaterials ?? []).filter((x) => x.id !== m.id)
              for (const e of t.elements) {
                if ((e.kind === 'wall' || e.kind === 'heatSource') && e.material === m.id)
                  e.material = null
              }
            })
          }
        >
          <Trash2 />
        </button>
      </div>
      <TextField
        label="Name"
        value={m.name}
        onCommit={(v) => set('Material name', (x) => void (x.name = v))}
      />
      <NumberField
        label="Conductivity"
        unit="W/(m·K)"
        value={m.thermalConductivity}
        rules={{ positive: true }}
        onCommit={(v) => set('Conductivity', (x) => void (x.thermalConductivity = v))}
      />
      <NumberField
        label="Density"
        unit="kg/m³"
        value={m.density}
        rules={{ positive: true }}
        onCommit={(v) => set('Density', (x) => void (x.density = v))}
      />
      <NumberField
        label="Specific heat"
        unit="J/(kg·K)"
        value={m.specificHeat}
        rules={{ positive: true }}
        onCommit={(v) => set('Specific heat', (x) => void (x.specificHeat = v))}
      />
    </div>
  )
}

/** A small "add from the library" menu as a native select whose first option is the label. */
function AddFrom(props: {
  label: string
  options: readonly (readonly [string, string])[]
  onPick: (key: string) => void
  disabled?: boolean
  testId?: string
}) {
  return (
    <span className="relative inline-flex items-center">
      <Plus className="pointer-events-none absolute left-1.5 size-3.5 text-muted-foreground" />
      <select
        data-testid={props.testId}
        disabled={props.disabled}
        className="h-6 rounded-md border border-border bg-background pr-1 pl-6 text-xs"
        value=""
        onChange={(e) => props.onPick(e.target.value)}
      >
        <option value="" disabled>
          {props.label}
        </option>
        {props.options.map(([k, name]) => (
          <option key={k} value={k}>
            {name}
          </option>
        ))}
      </select>
    </span>
  )
}

const REFERENCE_FIELDS = [
  ['length', 'Length L', 'm'],
  ['velocity', 'Velocity U', 'm/s'],
  ['temperatureDifference', 'ΔT', 'K'],
] as const

/**
 * The scales of Re, Ra and (in the viewer) C_D, C_L, St and Nu, e.g. a cylinder's diameter and
 * the mean inlet velocity. Empty fields use the solver's characteristic scales, shown greyed.
 */
function ReferenceGroup() {
  const reference = useEditor((s) => s.table.reference)
  const thermal = useEditor((s) => s.table.physics.thermal)
  const defaults = useEditor((s) => currentValidation(s.validation)?.estimate?.numbers.reference)
  return (
    <Group title="Reference scales">
      <p className="text-[11px] text-muted-foreground">
        For the reported numbers only (Re, Ra, force coefficients, St, Nu); the time step does not
        change. Empty: the solver's characteristic scale.
      </p>
      {REFERENCE_FIELDS.filter(([key]) => thermal || key !== 'temperatureDifference').map(
        ([key, label, unit]) => (
          <NumberField
            key={key}
            label={label}
            unit={unit}
            optional
            value={reference?.[key] ?? null}
            placeholder={
              reference?.[key] == null && defaults ? formatValue(defaults[key]) : undefined
            }
            rules={{ positive: true }}
            testId={`reference-${key}`}
            onCommit={(v) =>
              update(`Reference ${label}`, (t) => {
                const next: ReferenceScales = { ...t.reference, [key]: v }
                t.reference = referenceOrNull(next)
              })
            }
          />
        ),
      )}
    </Group>
  )
}

export function ScenePropertiesPanel() {
  const table = useEditor((s) => s.table)
  const { fluids, initial, physics } = table
  const materials = table.solidMaterials ?? []
  const why = (f: Fluid): string | null => {
    if (fluids.length === 1) return 'A scene needs at least one fluid'
    if (initial.fluid === f.id) return 'Used by the initial conditions'
    if (table.elements.some((e) => e.kind === 'inlet' && e.fluid === f.id))
      return 'Used by an inlet'
    return null
  }
  return (
    <div className="flex flex-col gap-5" data-testid="scene-panel">
      <Group title="Scene">
        <TextField
          label="Name"
          value={table.name}
          testId="scene-name-field"
          onCommit={(v) => update('Scene name', (t) => void (t.name = v))}
        />
        <TextField
          label="Description"
          multiline
          value={table.description ?? ''}
          onCommit={(v) =>
            update('Description', (t) => void (t.description = v.trim() === '' ? null : v))
          }
        />
      </Group>
      <GridGroup />
      <EdgesGroup />
      <Group
        title="Fluids"
        actions={
          <AddFrom
            label="Add"
            testId="add-fluid"
            disabled={nextId(fluids) === null}
            options={FLUID_PRESETS.map((p) => [p.key, p.name] as const)}
            onPick={(key) => {
              const p = FLUID_PRESETS.find((x) => x.key === key)!
              update(`Add ${p.name}`, (t) => void t.fluids.push(presetFluid(p, nextId(t.fluids)!)))
            }}
          />
        }
      >
        {fluids.map((f) => (
          <FluidEditor key={f.id} fluid={f} removable={why(f)} thermal={physics.thermal} />
        ))}
      </Group>
      <Group
        title="Solid materials"
        actions={
          <AddFrom
            label="Add"
            disabled={nextId(materials) === null}
            options={SOLID_PRESETS.map((p) => [p.key, p.name] as const)}
            onPick={(key) => {
              const p = SOLID_PRESETS.find((x) => x.key === key)!
              update(`Add ${p.name}`, (t) => {
                t.solidMaterials = [
                  ...(t.solidMaterials ?? []),
                  presetSolid(p, nextId(t.solidMaterials ?? [])!),
                ]
              })
            }}
          />
        }
      >
        {materials.length === 0 && (
          <p className="text-[11px] text-muted-foreground">None: walls are not conducting.</p>
        )}
        {materials.map((m) => (
          <SolidEditor key={m.id} material={m} />
        ))}
      </Group>
      <Group title="Initial conditions">
        <SelectField
          label="Fluid"
          value={String(initial.fluid)}
          options={fluids.map((f) => [String(f.id), f.name] as const)}
          onChange={(v) => update('Initial fluid', (t) => void (t.initial.fluid = Number(v)))}
        />
        <VectorField
          label="Velocity"
          unit="m/s"
          value={initial.velocity}
          onCommit={(v) => update('Initial velocity', (t) => void (t.initial.velocity = v))}
        />
        <NumberField
          label="Temperature"
          unit="K"
          value={initial.temperature}
          rules={{ positive: true }}
          onCommit={(v) => update('Initial temperature', (t) => void (t.initial.temperature = v))}
        />
        <NumberField
          label="Gauge pressure"
          unit="Pa"
          value={initial.pressure}
          onCommit={(v) => update('Initial pressure', (t) => void (t.initial.pressure = v))}
        />
      </Group>
      <Group title="Physics">
        <VectorField
          label="Gravity"
          unit="m/s²"
          value={physics.gravity}
          onCommit={(v) => update('Gravity', (t) => void (t.physics.gravity = v))}
        />
        <CheckField
          label="Temperature"
          checked={physics.thermal}
          hint="solve the temperature field"
          testId="physics-thermal"
          onChange={(v) =>
            update(v ? 'Thermal on' : 'Thermal off', (t) => {
              t.physics.thermal = v
              // Fluids need their thermal properties: fill them from the preset or air's.
              if (v) {
                for (const f of t.fluids) {
                  const p = FLUID_PRESETS.find((x) => x.key === f.preset) ?? FLUID_PRESETS[0]
                  f.thermalConductivity ??= p.thermalConductivity
                  f.specificHeat ??= p.specificHeat
                  f.thermalExpansion ??= p.thermalExpansion
                }
              }
            })
          }
        />
        <SelectField
          label="Buoyancy"
          value={physics.buoyancy}
          options={[
            ['none', 'None'],
            ['boussinesq', 'Boussinesq'],
          ]}
          onChange={(v) => update('Buoyancy', (t) => void (t.physics.buoyancy = v))}
        />
        <CheckField
          label="Free surface"
          checked={physics.freeSurface ?? false}
          hint="liquid with empty (gas) cells"
          onChange={(v) =>
            update(
              v ? 'Free surface on' : 'Free surface off',
              (t) => void (t.physics.freeSurface = v),
            )
          }
        />
      </Group>
      <ReferenceGroup />
    </div>
  )
}
