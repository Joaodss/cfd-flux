// Conversion between a `Scene` and the editor's document: the scene table (everything but the
// layers, kept in the store) and the pixel layers (kept outside React).
import type { ErrorObject, ValidateFunction } from 'ajv'
import { Layers } from '../editor/layers.ts'
import type { LayerData2, ReferenceScales, Scene } from '../generated/scene.ts'
import { decodeU16, decodeU8, encodeU16, encodeU8 } from './codec.ts'

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
export async function fromScene(scene: Scene): Promise<Document> {
  const { layers: data, ...table } = scene
  const { width, height } = scene.grid
  const cells = width * height
  const [cellType, elementId] = await Promise.all([
    decodeU8(data.cellType, cells, 'cellType'),
    decodeU16(data.elementId, cells, 'elementId'),
  ])
  return {
    table,
    layers: new Layers(width, height, cellType, elementId),
    fluidId: data.fluidId ?? null,
  }
}

/** The document as a scene, its layers encoded as `deflate+base64`. */
export async function toScene(doc: Document): Promise<Scene> {
  const [cellType, elementId] = await Promise.all([
    encodeU8(doc.layers.cellType),
    encodeU16(doc.layers.elementId),
  ])
  const layers: Scene['layers'] = { cellType, elementId }
  if (doc.fluidId) layers.fluidId = doc.fluidId
  // Same key order as the Rust serializer, so saved files read like the CLI's.
  const { schemaVersion, name, description, grid, fluids, solidMaterials, elements, probes } =
    doc.table
  const reference = referenceOrNull(doc.table.reference)
  return {
    schemaVersion,
    name,
    ...(description ? { description } : {}),
    grid,
    fluids,
    solidMaterials: solidMaterials ?? [],
    elements,
    probes: probes ?? [],
    initial: doc.table.initial,
    physics: doc.table.physics,
    ...(reference ? { reference } : {}),
    run: doc.table.run,
    layers,
  }
}

/** The reference scales that are set (keys in the Rust order), or `null` when none is. */
export function referenceOrNull(r: ReferenceScales | null | undefined): ReferenceScales | null {
  if (!r) return null
  const out: ReferenceScales = {}
  if (r.length != null) out.length = r.length
  if (r.velocity != null) out.velocity = r.velocity
  if (r.temperatureDifference != null) out.temperatureDifference = r.temperatureDifference
  return Object.keys(out).length > 0 ? out : null
}

/**
 * The JSON Schema check, loaded and compiled on first use: ajv and the schema are a separate chunk
 * (≈ 120 kB) that only opening a file needs.
 */
let validator: Promise<ValidateFunction> | null = null
function schemaValidator(): Promise<ValidateFunction> {
  validator ??= Promise.all([
    import('ajv/dist/2020.js'),
    import('../../../../schema/scene.schema.json', { with: { type: 'json' } }),
  ]).then(([{ Ajv2020 }, schema]) =>
    new Ajv2020({ allErrors: true, strict: false }).compile(schema.default),
  )
  return validator
}

function schemaErrors(errors: ErrorObject[] | null | undefined): string {
  return (errors ?? [])
    .slice(0, 3)
    .map((e) => `${e.instancePath || '(scene)'} ${e.message ?? 'is invalid'}`)
    .join('; ')
}

/**
 * Reads a scene file (`.cfdscene` or `.json`): JSON, then the JSON Schema generated from Rust,
 * then the layers. Throws a readable message at the first failure.
 */
export async function parseScene(text: string): Promise<Document> {
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch (e) {
    throw new Error(`not a JSON file: ${(e as Error).message}`)
  }
  const validate = await schemaValidator()
  if (!validate(json)) throw new Error(`not a cfd-flux scene: ${schemaErrors(validate.errors)}`)
  return fromScene(json as Scene)
}
