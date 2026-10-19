// Boundary elements: defaults for new ones, ids and names, the palette's kinds, cell counts.
import type { Element } from '../generated/scene.ts'
import type { SceneTable } from '../scene/io.ts'
import type { ElementKind } from './colours.ts'
import { CellType, type Layers } from './layers.ts'
import type { Paint } from './tools/tools.ts'

/** What the palette paints: a cell type, an element kind, or probes (a point list). */
export type PaletteKind = 'fluid' | ElementKind | 'empty' | 'probe'

export const ELEMENT_KINDS: ElementKind[] = ['wall', 'inlet', 'outlet', 'heatSource']

export const KIND_NAMES: Record<PaletteKind, string> = {
  fluid: 'Fluid',
  wall: 'Wall',
  inlet: 'Inlet',
  outlet: 'Outlet',
  heatSource: 'Heat source',
  empty: 'Empty (gas)',
  probe: 'Probe',
}

export function isElementKind(k: PaletteKind): k is ElementKind {
  return (ELEMENT_KINDS as string[]).includes(k)
}

/** The largest element id the `u16` layer can hold. */
export const MAX_ELEMENT_ID = 65535

/** The next free element id (one past the largest), or null when the ids are exhausted. */
export function nextElementId(elements: readonly Element[]): number | null {
  const max = elements.reduce((m, e) => Math.max(m, e.id), 0)
  return max < MAX_ELEMENT_ID ? max + 1 : null
}

/** `Wall 3` style name: one more than the largest number already used for this kind's name. */
export function nextName(elements: readonly Element[], kind: ElementKind): string {
  const base = KIND_NAMES[kind]
  const pattern = new RegExp(`^${base} (\\d+)$`)
  let n = 0
  for (const e of elements) {
    const m = e.name ? pattern.exec(e.name) : null
    if (m) n = Math.max(n, Number(m[1]))
  }
  return `${base} ${n + 1}`
}

/** A new element of `kind` with the defaults of the design proposal (§4). */
export function defaultElement(table: SceneTable, kind: ElementKind, id: number): Element {
  const name = nextName(table.elements, kind)
  switch (kind) {
    case 'wall':
      return { id, name, kind, velocity: { type: 'noSlip' }, thermal: { type: 'adiabatic' } }
    case 'inlet':
      return {
        id,
        name,
        kind,
        velocity: { type: 'uniform', value: [0.1, 0] },
        thermal: { type: 'adiabatic' },
        fluid: table.initial.fluid,
      }
    case 'outlet':
      return { id, name, kind, pressure: { type: 'pressure', value: 0 } }
    case 'heatSource':
      return { id, name, kind, thermal: { type: 'fixed', value: 350 } }
  }
}

export function elementLabel(e: Element): string {
  return e.name ?? `${KIND_NAMES[e.kind]} ${e.id}`
}

/** The paint of a palette kind that needs no element (null for element kinds and probes). */
export function plainPaint(kind: PaletteKind): Paint | null {
  if (kind === 'fluid') return { cellType: CellType.Fluid, elementId: 0 }
  if (kind === 'empty') return { cellType: CellType.Empty, elementId: 0 }
  return null
}

/** The active element of a kind if it still exists with that kind (undo may have removed it). */
export function validElement(
  elements: readonly Element[],
  kind: ElementKind,
  id: number | null,
): number | null {
  return id !== null && elements.some((e) => e.id === id && e.kind === kind) ? id : null
}

/** Number of cells per element id (index = id), in one pass over the layer. */
export function cellCounts(layers: Layers): Uint32Array {
  const counts = new Uint32Array(MAX_ELEMENT_ID + 1)
  const ids = layers.elementId
  for (let i = 0; i < ids.length; i++) counts[ids[i]]++
  return counts
}

let countsKey: [Layers, number] | null = null
let countsValue: Uint32Array = new Uint32Array(0)

/**
 * `cellCounts`, recomputed only when the layers or `version` change (the history version: counts
 * change with operations, not on every pointer move of a drag).
 */
export function cachedCellCounts(layers: Layers, version: number): Uint32Array {
  if (!countsKey || countsKey[0] !== layers || countsKey[1] !== version) {
    countsValue = cellCounts(layers)
    countsKey = [layers, version]
  }
  return countsValue
}
