// Conversion between a `Scene` and the editor's document: the scene table (everything but the
// layers, kept in the store) and the pixel layers (kept outside React).
import { Layers } from '../editor/layers.ts'
import type { LayerData2, Scene } from '../generated/scene.ts'
import { decodeU16, decodeU8 } from './codec.ts'

/** Everything in a scene except its pixel layers. */
export type SceneTable = Omit<Scene, 'layers'>

export interface Document {
  table: SceneTable
  layers: Layers
  /**
   * The optional `fluidId` layer, kept as it was loaded: the editor does not paint it yet
   * (multi-fluid scenes come with Phase 6b), but saving must not drop it.
   */
  fluidId: LayerData2 | null
}

/** Splits a scene into its table and decoded layers. Throws with a readable message on bad layers. */
export function fromScene(scene: Scene): Document {
  const { layers: data, ...table } = scene
  const { width, height } = scene.grid
  const cells = width * height
  const layers = new Layers(
    width,
    height,
    decodeU8(data.cellType, cells, 'cellType'),
    decodeU16(data.elementId, cells, 'elementId'),
  )
  return { table, layers, fluidId: data.fluidId ?? null }
}
