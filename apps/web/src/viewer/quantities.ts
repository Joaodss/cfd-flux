// What the viewer can colour: the frame's fields and the scalars derived from them (|u|, u_x,
// u_y from the velocity), with their units and default colour maps (FR-20).
import type { FieldDesc } from '../frames/lffr.ts'

/** How a quantity is read from its field: the value, or the magnitude / a component of a vector. */
export type Component = 'scalar' | 'magnitude' | 'x' | 'y'

export interface Quantity {
  /** `velocity` (its magnitude, as the server's ranges), `velocity.x`, `pressure`, … */
  key: string
  label: string
  unit: string
  field: string
  component: Component
  colormap: string
  /** Diverging quantity: by default a range symmetric around zero. */
  symmetric: boolean
}

const SCALARS: Record<string, Omit<Quantity, 'key' | 'field' | 'component'>> = {
  pressure: { label: 'Pressure (gauge)', unit: 'Pa', colormap: 'coolwarm', symmetric: true },
  vorticity: { label: 'Vorticity', unit: '1/s', colormap: 'coolwarm', symmetric: true },
  temperature: { label: 'Temperature', unit: 'K', colormap: 'inferno', symmetric: false },
  density: { label: 'Density', unit: 'kg/m³', colormap: 'viridis', symmetric: false },
  fillFraction: { label: 'Fill fraction', unit: '', colormap: 'greys', symmetric: false },
}

/** The quantities of a frame's fields, in field order. */
export function quantitiesOf(fields: readonly FieldDesc[]): Quantity[] {
  return fields.flatMap((f): Quantity[] => {
    if (f.components === 2) {
      const base = { field: f.name, unit: 'm/s', symmetric: false }
      const label = f.name === 'velocity' ? 'Velocity' : f.name
      return [
        {
          ...base,
          key: f.name,
          label: `${label} |u|`,
          component: 'magnitude',
          colormap: 'viridis',
        },
        {
          ...base,
          key: `${f.name}.x`,
          label: `${label} u_x`,
          component: 'x',
          colormap: 'blueRed',
          symmetric: true,
        },
        {
          ...base,
          key: `${f.name}.y`,
          label: `${label} u_y`,
          component: 'y',
          colormap: 'blueRed',
          symmetric: true,
        },
      ]
    }
    const known = SCALARS[f.name] ?? {
      label: f.name,
      unit: '',
      colormap: 'viridis',
      symmetric: false,
    }
    return [{ ...known, key: f.name, field: f.name, component: 'scalar' }]
  })
}

/** The value of a quantity from a cell's (or block's) components. */
export function componentValue(component: Component, a: number, b: number): number {
  switch (component) {
    case 'scalar':
    case 'x':
      return a
    case 'y':
      return b
    case 'magnitude':
      return Math.hypot(a, b)
  }
}
