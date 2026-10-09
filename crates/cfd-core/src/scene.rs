//! The `Scene` exchange format, version 1.
//!
//! These types are the source of truth for the format: `schema/scene.schema.json` is generated
//! from them (`cfd-cli schema`) and the frontend TypeScript types are generated from that schema.
//! All quantities are in SI units; conversion to lattice units happens in the solver.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::layers::LayerData;

/// Current version of the scene format.
pub const SCHEMA_VERSION: u32 = 1;

/// A complete simulation problem: geometry (pixel layers), materials, boundary conditions and
/// run settings. Describes the physics, not the algorithm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(title = "Scene")]
pub struct Scene {
    /// Format version; must equal the version supported by the server.
    pub schema_version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub grid: Grid,
    pub fluids: Vec<Fluid>,
    #[serde(default)]
    pub solid_materials: Vec<SolidMaterial>,
    /// Boundary elements referenced by the `elementId` layer.
    pub elements: Vec<Element>,
    #[serde(default)]
    pub probes: Vec<Probe>,
    pub initial: InitialConditions,
    pub physics: Physics,
    pub run: RunConfig,
    pub layers: Layers,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grid {
    /// Number of cells along x.
    pub width: u32,
    /// Number of cells along y.
    pub height: u32,
    /// Cell size in metres.
    pub cell_size: f64,
    /// Behaviour of the four domain edges where no element is drawn.
    #[serde(default)]
    pub edges: DomainEdges,
}

impl Grid {
    pub fn cell_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DomainEdges {
    pub left: EdgeKind,
    pub right: EdgeKind,
    pub bottom: EdgeKind,
    pub top: EdgeKind,
}

impl Default for DomainEdges {
    fn default() -> Self {
        Self {
            left: EdgeKind::Wall,
            right: EdgeKind::Wall,
            bottom: EdgeKind::Wall,
            top: EdgeKind::Wall,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    /// No-slip, adiabatic wall.
    Wall,
    /// Periodic with the opposite edge (both edges must be periodic).
    Periodic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fluid {
    /// Referenced by `fluidId` layer values and by elements. Must be >= 1.
    pub id: u8,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// kg/m³
    pub density: f64,
    /// m²/s
    pub kinematic_viscosity: f64,
    /// W/(m·K)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_conductivity: Option<f64>,
    /// J/(kg·K)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specific_heat: Option<f64>,
    /// 1/K (Boussinesq coefficient)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_expansion: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SolidMaterial {
    /// Must be >= 1.
    pub id: u8,
    pub name: String,
    /// W/(m·K)
    pub thermal_conductivity: f64,
    /// kg/m³
    pub density: f64,
    /// J/(kg·K)
    pub specific_heat: f64,
}

/// A boundary element: every cell carrying its id in the `elementId` layer shares its properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    /// Must be >= 1 and unique.
    pub id: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub kind: ElementKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ElementKind {
    Wall {
        velocity: WallVelocity,
        thermal: ThermalBc,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        material: Option<u8>,
    },
    Inlet {
        velocity: InletVelocity,
        thermal: ThermalBc,
        fluid: u8,
    },
    Outlet {
        pressure: OutletBc,
    },
    HeatSource {
        thermal: ThermalBc,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        material: Option<u8>,
    },
}

impl ElementKind {
    /// The `cellType` value that cells of this element must have.
    pub fn cell_type(&self) -> CellType {
        match self {
            ElementKind::Wall { .. } => CellType::Solid,
            ElementKind::Inlet { .. } => CellType::Inlet,
            ElementKind::Outlet { .. } => CellType::Outlet,
            ElementKind::HeatSource { .. } => CellType::HeatSource,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WallVelocity {
    NoSlip,
    Slip,
    /// Tangential wall velocity in m/s.
    Moving {
        velocity: [f64; 2],
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InletVelocity {
    /// Same velocity (m/s) on every inlet cell.
    Uniform { value: [f64; 2] },
    /// Parabolic profile across the inlet with the given peak velocity (m/s).
    Parabolic { peak: [f64; 2] },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OutletBc {
    /// Gauge pressure in Pa.
    Pressure {
        value: f64,
    },
    ZeroGradient,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ThermalBc {
    Adiabatic,
    /// Temperature in K.
    Fixed {
        value: f64,
    },
    /// Heat flux in W/m² (positive into the fluid).
    Flux {
        value: f64,
    },
    /// Convective exchange: h in W/(m²·K), ambient temperature in K.
    Convective {
        h: f64,
        ambient: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Probe {
    pub name: String,
    /// Cell coordinates `[x, y]`, origin at the bottom-left corner.
    pub position: [u32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InitialConditions {
    /// Fluid used where the `fluidId` layer is 0 (or absent).
    pub fluid: u8,
    /// m/s
    pub velocity: [f64; 2],
    /// K
    pub temperature: f64,
    /// Gauge pressure in Pa.
    pub pressure: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Physics {
    /// m/s²
    pub gravity: [f64; 2],
    /// Solve the temperature field.
    pub thermal: bool,
    pub buoyancy: Buoyancy,
    /// Liquid with a free surface; `empty` cells are gas/void.
    #[serde(default)]
    pub free_surface: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Buoyancy {
    None,
    Boussinesq,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunConfig {
    pub method: Method,
    pub backend: BackendPreference,
    pub precision: Precision,
    /// Simulated physical time in seconds.
    pub end_time: f64,
    /// Physical time between output frames in seconds.
    pub output_interval: f64,
    pub output_fields: Vec<OutputField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    Lbm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum BackendPreference {
    /// CUDA if available, then wgpu, then CPU.
    Auto,
    Cpu,
    Wgpu,
    Cuda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Precision {
    F32,
    F64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OutputField {
    Velocity,
    Pressure,
    Density,
    Temperature,
    Vorticity,
    FillFraction,
}

/// Per-cell layers, row-major from the bottom-left corner (`index = y * width + x`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layers {
    /// `u8` values of [`CellType`].
    pub cell_type: LayerData,
    /// `u16` element ids; 0 = none.
    pub element_id: LayerData,
    /// Optional `u8` initial fluid per cell; 0 = `initial.fluid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fluid_id: Option<LayerData>,
}

/// Values of the `cellType` layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CellType {
    Fluid = 0,
    Solid = 1,
    Inlet = 2,
    Outlet = 3,
    HeatSource = 4,
    /// Gas/void above a free surface.
    Empty = 5,
}

impl TryFrom<u8> for CellType {
    type Error = u8;

    fn try_from(v: u8) -> Result<Self, u8> {
        Ok(match v {
            0 => CellType::Fluid,
            1 => CellType::Solid,
            2 => CellType::Inlet,
            3 => CellType::Outlet,
            4 => CellType::HeatSource,
            5 => CellType::Empty,
            other => return Err(other),
        })
    }
}

impl Scene {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("Scene serialization cannot fail")
    }

    /// JSON Schema of the format, as written to `schema/scene.schema.json`.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Scene)).expect("schema is valid JSON")
    }
}
