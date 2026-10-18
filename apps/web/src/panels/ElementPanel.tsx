// The Element tab: the selected boundary element's properties. The form follows the element's
// kind and each condition's `type` (the generated discriminated unions), so only the fields that
// type has are shown.
import type { Draft } from 'immer'
import { Group, NumberField, SelectField, TextField, VectorField } from '../components/fields.tsx'
import { coloursFor, rgbaCss } from '../editor/colours.ts'
import { cachedCellCounts, KIND_NAMES } from '../editor/elements.ts'
import { useEditor } from '../editor/store.ts'
import type {
  Element,
  InletVelocity,
  OutletBc,
  ThermalBc,
  WallVelocity,
} from '../generated/scene.ts'

type Options<T extends string> = readonly (readonly [T, string])[]

const WALL_VELOCITY: Options<WallVelocity['type']> = [
  ['noSlip', 'No slip'],
  ['slip', 'Slip'],
  ['moving', 'Moving wall'],
]
const INLET_VELOCITY: Options<InletVelocity['type']> = [
  ['uniform', 'Uniform velocity'],
  ['parabolic', 'Parabolic profile'],
]
const OUTLET: Options<OutletBc['type']> = [
  ['pressure', 'Fixed pressure'],
  ['zeroGradient', 'Zero gradient'],
]
const THERMAL: Options<ThermalBc['type']> = [
  ['adiabatic', 'Adiabatic'],
  ['fixed', 'Fixed temperature'],
  ['flux', 'Heat flux'],
  ['convective', 'Convective'],
]

/** A thermal condition of a new type, starting from sensible values. */
function thermalOf(type: ThermalBc['type'], ambient: number): ThermalBc {
  switch (type) {
    case 'adiabatic':
      return { type }
    case 'fixed':
      return { type, value: ambient }
    case 'flux':
      return { type, value: 0 }
    case 'convective':
      return { type, h: 10, ambient }
  }
}

function speedOf(v: WallVelocity | InletVelocity): [number, number] {
  if ('velocity' in v) return v.velocity
  if ('value' in v) return v.value
  if ('peak' in v) return v.peak
  return [0, 0]
}

/** Edits the selected element as one history entry. */
function edit(id: number, label: string, recipe: (e: Draft<Element>) => void): void {
  useEditor.getState().updateTable(label, (t) => {
    const e = t.elements.find((x) => x.id === id)
    if (e) recipe(e)
  })
}

function ThermalFields(props: { element: Element & { thermal: ThermalBc } }) {
  const { id, thermal } = props.element
  const ambient = useEditor((s) => s.table.initial.temperature)
  const thermalOn = useEditor((s) => s.table.physics.thermal)
  return (
    <Group title="Thermal">
      <SelectField
        label="Condition"
        value={thermal.type}
        options={THERMAL}
        testId="element-thermal"
        title={thermalOn ? undefined : 'Used when the scene solves temperature (Scene tab)'}
        onChange={(type) =>
          edit(id, 'Thermal condition', (e) => {
            if ('thermal' in e) e.thermal = thermalOf(type, ambient)
          })
        }
      />
      {(thermal.type === 'fixed' || thermal.type === 'flux') && (
        <NumberField
          label={thermal.type === 'fixed' ? 'Temperature' : 'Heat flux'}
          unit={thermal.type === 'fixed' ? 'K' : 'W/m²'}
          value={thermal.value}
          rules={thermal.type === 'fixed' ? { positive: true } : undefined}
          onCommit={(v) =>
            edit(id, 'Thermal value', (e) => {
              if ('thermal' in e && 'value' in e.thermal) e.thermal.value = v
            })
          }
        />
      )}
      {thermal.type === 'convective' && (
        <>
          <NumberField
            label="Coefficient h"
            unit="W/(m²·K)"
            value={thermal.h}
            rules={{ positive: true }}
            onCommit={(v) =>
              edit(id, 'Heat transfer coefficient', (e) => {
                if ('thermal' in e && e.thermal.type === 'convective') e.thermal.h = v
              })
            }
          />
          <NumberField
            label="Ambient"
            unit="K"
            value={thermal.ambient}
            rules={{ positive: true }}
            onCommit={(v) =>
              edit(id, 'Ambient temperature', (e) => {
                if ('thermal' in e && e.thermal.type === 'convective') e.thermal.ambient = v
              })
            }
          />
        </>
      )}
      {!thermalOn && (
        <p className="text-[11px] text-muted-foreground">
          Temperature is off for this scene (Scene → Physics).
        </p>
      )}
    </Group>
  )
}

function MaterialField(props: { element: Element & { material?: number | null } }) {
  const materials = useEditor((s) => s.table.solidMaterials ?? [])
  const { id, material } = props.element
  const options: Options<string> = [
    ['', 'None'],
    ...materials.map((m) => [String(m.id), m.name] as const),
  ]
  return (
    <SelectField
      label="Material"
      value={material == null ? '' : String(material)}
      options={options}
      title="Solid material (for conduction in solids; add materials in the Scene tab)"
      onChange={(v) =>
        edit(id, 'Material', (e) => {
          if (e.kind === 'wall' || e.kind === 'heatSource') e.material = v === '' ? null : Number(v)
        })
      }
    />
  )
}

export function ElementPanel() {
  const selected = useEditor((s) => s.selectedElement)
  const elements = useEditor((s) => s.table.elements)
  const fluids = useEditor((s) => s.table.fluids)
  const layers = useEditor((s) => s.layers)
  const historyVersion = useEditor((s) => s.historyVersion)
  const element = elements.find((e) => e.id === selected)
  if (!element) {
    return (
      <p className="text-xs text-muted-foreground">
        Select an element in the palette, or draw a wall, inlet, outlet or heat source.
      </p>
    )
  }
  const { id } = element
  const colour = coloursFor(elements).elements.get(id)!
  const count = cachedCellCounts(layers, historyVersion)[id]

  return (
    <div className="flex flex-col gap-4" data-testid="element-panel">
      <div className="flex items-center gap-2 text-xs">
        <span
          className="size-3.5 rounded-sm border border-border"
          style={{ background: rgbaCss(colour) }}
        />
        <span className="font-medium">{KIND_NAMES[element.kind]}</span>
        <span className="text-muted-foreground">
          id {id} · {count.toLocaleString('en')} cells
        </span>
      </div>
      <TextField
        label="Name"
        value={element.name ?? ''}
        testId="element-name"
        onCommit={(v) =>
          edit(id, 'Rename', (e) => {
            e.name = v.trim() === '' ? null : v.trim()
          })
        }
      />

      {element.kind === 'wall' && (
        <Group title="Velocity">
          <SelectField
            label="Condition"
            value={element.velocity.type}
            options={WALL_VELOCITY}
            onChange={(type) =>
              edit(id, 'Wall condition', (e) => {
                if (e.kind !== 'wall') return
                const v = speedOf(e.velocity)
                e.velocity = type === 'moving' ? { type, velocity: v } : { type }
              })
            }
          />
          {element.velocity.type === 'moving' && (
            <VectorField
              label="Wall velocity"
              unit="m/s"
              value={element.velocity.velocity}
              onCommit={(v) =>
                edit(id, 'Wall velocity', (e) => {
                  if (e.kind === 'wall' && e.velocity.type === 'moving') e.velocity.velocity = v
                })
              }
            />
          )}
        </Group>
      )}

      {element.kind === 'inlet' && (
        <Group title="Velocity">
          <SelectField
            label="Profile"
            value={element.velocity.type}
            options={INLET_VELOCITY}
            testId="inlet-profile"
            onChange={(type) =>
              edit(id, 'Inlet profile', (e) => {
                if (e.kind !== 'inlet') return
                const v = speedOf(e.velocity)
                e.velocity = type === 'uniform' ? { type, value: v } : { type, peak: v }
              })
            }
          />
          <VectorField
            label={element.velocity.type === 'uniform' ? 'Velocity' : 'Peak velocity'}
            unit="m/s"
            testId="inlet-velocity"
            value={speedOf(element.velocity)}
            onCommit={(v) =>
              edit(id, 'Inlet velocity', (e) => {
                if (e.kind !== 'inlet') return
                if (e.velocity.type === 'uniform') e.velocity.value = v
                else e.velocity.peak = v
              })
            }
          />
          <SelectField
            label="Fluid"
            value={String(element.fluid)}
            options={fluids.map((f) => [String(f.id), f.name] as const)}
            onChange={(v) =>
              edit(id, 'Inlet fluid', (e) => {
                if (e.kind === 'inlet') e.fluid = Number(v)
              })
            }
          />
        </Group>
      )}

      {element.kind === 'outlet' && (
        <Group title="Outlet">
          <SelectField
            label="Condition"
            value={element.pressure.type}
            options={OUTLET}
            onChange={(type) =>
              edit(id, 'Outlet condition', (e) => {
                if (e.kind === 'outlet') {
                  e.pressure = type === 'pressure' ? { type, value: 0 } : { type }
                }
              })
            }
          />
          {element.pressure.type === 'pressure' && (
            <NumberField
              label="Gauge pressure"
              unit="Pa"
              value={element.pressure.value}
              onCommit={(v) =>
                edit(id, 'Outlet pressure', (e) => {
                  if (e.kind === 'outlet' && e.pressure.type === 'pressure') e.pressure.value = v
                })
              }
            />
          )}
        </Group>
      )}

      {element.kind !== 'outlet' && <ThermalFields element={element} />}
      {(element.kind === 'wall' || element.kind === 'heatSource') && (
        <MaterialField element={element} />
      )}
    </div>
  )
}
