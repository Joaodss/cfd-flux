// Which elements of a run its force plots and statistics are about.
//
// `forces.csv` has a force for every element with a boundary link: the channel's walls, the
// inlet and the outlet as well as the cylinder. Only a body in the flow has a meaningful drag and
// lift coefficient, so the viewer starts on the first *body* — a wall element none of whose cells
// lies on the domain's edge — and offers the others in a list.
import type { Scene } from '../generated/scene.ts'

/** `cfd-job`'s column-safe name: ASCII letters, digits and `-` kept, anything else `_`. */
export function columnName(s: string): string {
  return s.replace(/[^A-Za-z0-9-]/g, '_')
}

/** The prefix of an element's columns in `forces.csv` (`{name}.fx_N_m`). */
export function elementColumn(e: { id: number; name?: string | null }): string {
  return columnName(e.name ?? `element${e.id}`)
}

/** The columns of the wall elements that do not touch the domain's edge, in element order. */
export function bodies(
  elements: Scene['elements'],
  elementId: Uint16Array,
  width: number,
  height: number,
): string[] {
  const onEdge = new Set<number>()
  const mark = (x: number, y: number) => onEdge.add(elementId[y * width + x])
  for (let x = 0; x < width; x++) {
    mark(x, 0)
    mark(x, height - 1)
  }
  for (let y = 0; y < height; y++) {
    mark(0, y)
    mark(width - 1, y)
  }
  return elements.filter((e) => e.kind === 'wall' && !onEdge.has(e.id)).map(elementColumn)
}

/** The element shown: the one chosen if it has forces, else the first body, else the first. */
export function shownElement(
  available: string[],
  bodyColumns: string[],
  chosen: string | null,
): string | null {
  if (chosen && available.includes(chosen)) return chosen
  return bodyColumns.find((b) => available.includes(b)) ?? available[0] ?? null
}
