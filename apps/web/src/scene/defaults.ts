// The scene a "New" document starts from (design proposal §14 Q13).
import { Layers } from '../editor/layers.ts'
import { SCHEMA_VERSION } from './version.ts'
import type { Document } from './io.ts'
import { FLUID_PRESETS, presetFluid } from './library.ts'

export const NEW_SCENE = { width: 400, height: 200, cellSize: 0.005 }

/** An empty channel of air: every cell fluid, domain edges walls. */
export function newDocument(
  width = NEW_SCENE.width,
  height = NEW_SCENE.height,
  cellSize = NEW_SCENE.cellSize,
): Document {
  return {
    table: {
      schemaVersion: SCHEMA_VERSION,
      name: 'Untitled',
      grid: {
        width,
        height,
        cellSize,
        edges: { left: 'wall', right: 'wall', bottom: 'wall', top: 'wall' },
      },
      fluids: [
        presetFluid(
          FLUID_PRESETS.find((f) => f.key === 'air')!,
          1,
        ),
      ],
      solidMaterials: [],
      elements: [],
      probes: [],
      initial: { fluid: 1, velocity: [0, 0], temperature: 293.15, pressure: 0 },
      physics: { gravity: [0, 0], thermal: false, buoyancy: 'none', freeSurface: false },
      run: {
        method: 'lbm',
        backend: 'auto',
        precision: 'f32',
        endTime: 10,
        outputInterval: 0.05,
        outputFields: ['velocity', 'pressure', 'vorticity'],
      },
    },
    layers: new Layers(width, height),
    fluidId: null,
  }
}
