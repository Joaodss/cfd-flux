/* Generated from schema/scene.schema.json by scripts/gen-types.mjs. Do not edit. */

/**
 * A boundary element: every cell carrying its id in the `elementId` layer shares its properties.
 */
export type Element = {
  /**
   * Must be >= 1 and unique.
   */
  id: number
  name?: string | null
} & Element1
export type Element1 =
  | {
      kind: 'wall'
      material?: number | null
      thermal: ThermalBc
      velocity: WallVelocity
    }
  | {
      fluid: number
      kind: 'inlet'
      thermal: ThermalBc
      velocity: InletVelocity
    }
  | {
      kind: 'outlet'
      pressure: OutletBc
    }
  | {
      kind: 'heatSource'
      material?: number | null
      thermal: ThermalBc
    }
export type ThermalBc =
  | {
      type: 'adiabatic'
    }
  | {
      type: 'fixed'
      value: number
    }
  | {
      type: 'flux'
      value: number
    }
  | {
      ambient: number
      h: number
      type: 'convective'
    }
export type WallVelocity =
  | {
      type: 'noSlip'
    }
  | {
      type: 'slip'
    }
  | {
      type: 'moving'
      /**
       * @minItems 2
       * @maxItems 2
       */
      velocity: [number, number]
    }
export type InletVelocity =
  | {
      type: 'uniform'
      /**
       * @minItems 2
       * @maxItems 2
       */
      value: [number, number]
    }
  | {
      /**
       * @minItems 2
       * @maxItems 2
       */
      peak: [number, number]
      type: 'parabolic'
    }
export type OutletBc =
  | {
      type: 'pressure'
      value: number
    }
  | {
      type: 'zeroGradient'
    }
export type EdgeKind = 'wall' | 'periodic'
export type LayerDtype = 'u8' | 'u16'
export type LayerEncoding = 'raw+base64' | 'zstd+base64'
export type Buoyancy = 'none' | 'boussinesq'
export type BackendPreference = ('cpu' | 'wgpu' | 'cuda') | 'auto'
export type Method = 'lbm'
export type OutputField =
  'velocity' | 'pressure' | 'density' | 'temperature' | 'vorticity' | 'fillFraction'
export type Precision = 'f32' | 'f64'

/**
 * A complete simulation problem: geometry (pixel layers), materials, boundary conditions and
 * run settings. Describes the physics, not the algorithm.
 */
export interface Scene {
  description?: string | null
  /**
   * Boundary elements referenced by the `elementId` layer.
   */
  elements: Element[]
  fluids: Fluid[]
  grid: Grid
  initial: InitialConditions
  layers: Layers
  name: string
  physics: Physics
  probes?: Probe[]
  run: RunConfig
  /**
   * Format version; must equal the version supported by the server.
   */
  schemaVersion: number
  solidMaterials?: SolidMaterial[]
}
export interface Fluid {
  /**
   * kg/m³
   */
  density: number
  /**
   * Referenced by `fluidId` layer values and by elements. Must be >= 1.
   */
  id: number
  /**
   * m²/s
   */
  kinematicViscosity: number
  name: string
  preset?: string | null
  /**
   * J/(kg·K)
   */
  specificHeat?: number | null
  /**
   * W/(m·K)
   */
  thermalConductivity?: number | null
  /**
   * 1/K (Boussinesq coefficient)
   */
  thermalExpansion?: number | null
}
export interface Grid {
  /**
   * Cell size in metres.
   */
  cellSize: number
  edges?: DomainEdges
  /**
   * Number of cells along y.
   */
  height: number
  /**
   * Number of cells along x.
   */
  width: number
}
/**
 * Behaviour of the four domain edges where no element is drawn.
 */
export interface DomainEdges {
  bottom: EdgeKind
  left: EdgeKind
  right: EdgeKind
  top: EdgeKind
}
export interface InitialConditions {
  /**
   * Fluid used where the `fluidId` layer is 0 (or absent).
   */
  fluid: number
  /**
   * Gauge pressure in Pa.
   */
  pressure: number
  /**
   * K
   */
  temperature: number
  /**
   * m/s
   *
   * @minItems 2
   * @maxItems 2
   */
  velocity: [number, number]
}
/**
 * Per-cell layers, row-major from the bottom-left corner (`index = y * width + x`).
 */
export interface Layers {
  cellType: LayerData
  elementId: LayerData1
  /**
   * Optional `u8` initial fluid per cell; 0 = `initial.fluid`.
   */
  fluidId?: LayerData2 | null
}
/**
 * `u8` values of [`CellType`].
 */
export interface LayerData {
  data: string
  dtype: LayerDtype
  encoding: LayerEncoding
}
/**
 * `u16` element ids; 0 = none.
 */
export interface LayerData1 {
  data: string
  dtype: LayerDtype
  encoding: LayerEncoding
}
export interface LayerData2 {
  data: string
  dtype: LayerDtype
  encoding: LayerEncoding
}
export interface Physics {
  buoyancy: Buoyancy
  /**
   * Liquid with a free surface; `empty` cells are gas/void.
   */
  freeSurface?: boolean
  /**
   * m/s²
   *
   * @minItems 2
   * @maxItems 2
   */
  gravity: [number, number]
  /**
   * Solve the temperature field.
   */
  thermal: boolean
}
export interface Probe {
  name: string
  /**
   * Cell coordinates `[x, y]`, origin at the bottom-left corner.
   *
   * @minItems 2
   * @maxItems 2
   */
  position: [number, number]
}
export interface RunConfig {
  backend: BackendPreference
  /**
   * Simulated physical time in seconds.
   */
  endTime: number
  method: Method
  outputFields: OutputField[]
  /**
   * Physical time between output frames in seconds.
   */
  outputInterval: number
  precision: Precision
}
export interface SolidMaterial {
  /**
   * kg/m³
   */
  density: number
  /**
   * Must be >= 1.
   */
  id: number
  name: string
  /**
   * J/(kg·K)
   */
  specificHeat: number
  /**
   * W/(m·K)
   */
  thermalConductivity: number
}
