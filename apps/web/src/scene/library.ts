// The preset fluids and solid materials of `cfd_core::library`, generated into
// `src/generated/library.json` by `cfd-cli library` (a Rust test keeps it current).
import type { Fluid, SolidMaterial } from '../generated/scene.ts'
import library from '../generated/library.json' with { type: 'json' }

export type FluidPreset = (typeof library.fluids)[number]
export type SolidPreset = (typeof library.solids)[number]

export const FLUID_PRESETS: readonly FluidPreset[] = library.fluids
export const SOLID_PRESETS: readonly SolidPreset[] = library.solids

/** A preset as a scene fluid (keeps `id`). */
export function presetFluid(p: FluidPreset, id: number): Fluid {
  return {
    id,
    name: p.name,
    preset: p.key,
    density: p.density,
    kinematicViscosity: p.kinematicViscosity,
    thermalConductivity: p.thermalConductivity,
    specificHeat: p.specificHeat,
    thermalExpansion: p.thermalExpansion,
  }
}

/** A preset as a scene solid material (keeps `id`). */
export function presetSolid(p: SolidPreset, id: number): SolidMaterial {
  return {
    id,
    name: p.name,
    thermalConductivity: p.thermalConductivity,
    density: p.density,
    specificHeat: p.specificHeat,
  }
}

/** The next free id of a fluid or material list (`u8`: up to 255), or null when full. */
export function nextId(items: readonly { id: number }[]): number | null {
  const max = items.reduce((m, i) => Math.max(m, i.id), 0)
  return max < 255 ? max + 1 : null
}
