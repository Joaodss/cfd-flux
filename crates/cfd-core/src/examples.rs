//! Built-in example scenes, generated in code so they stay reproducible.
//!
//! `cfd-cli example --all scenes/` writes them to `scenes/`; a test checks the committed
//! files match. Geometries follow the validation cases in `docs/08-validation-guide.md`.

use crate::layers::{LayerData, LayerEncoding};
use crate::scene::*;
use crate::units::STANDARD_GRAVITY;

/// Names accepted by [`by_name`].
pub const NAMES: [&str; 4] = [
    "channel",
    "cylinder-re100",
    "heated-cavity-ra1e5",
    "dam-break",
];

pub fn by_name(name: &str) -> Option<Scene> {
    Some(match name {
        "channel" => channel(),
        "cylinder-re100" => cylinder_re100(),
        "heated-cavity-ra1e5" => heated_cavity_ra1e5(),
        "dam-break" => dam_break(),
        _ => return None,
    })
}

/// Pixel canvas used to draw scene geometry in code, mirroring what the web editor does.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub cell_type: Vec<u8>,
    pub element_id: Vec<u16>,
}

impl Canvas {
    pub fn new(width: u32, height: u32, fill: CellType) -> Self {
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            cell_type: vec![fill as u8; n],
            element_id: vec![0; n],
        }
    }

    pub fn set(&mut self, x: u32, y: u32, ct: CellType, element: u16) {
        let i = (y * self.width + x) as usize;
        self.cell_type[i] = ct as u8;
        self.element_id[i] = element;
    }

    /// Fills `[x0, x1) × [y0, y1)`.
    pub fn rect(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, ct: CellType, element: u16) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.set(x, y, ct, element);
            }
        }
    }

    /// Fills every cell whose centre lies inside the circle (centre in cell-edge coordinates).
    pub fn disk(&mut self, cx: f64, cy: f64, r: f64, ct: CellType, element: u16) {
        for y in 0..self.height {
            for x in 0..self.width {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                if dx * dx + dy * dy <= r * r {
                    self.set(x, y, ct, element);
                }
            }
        }
    }

    pub fn layers(&self) -> Layers {
        Layers {
            cell_type: LayerData::encode_u8(&self.cell_type, LayerEncoding::ZstdBase64),
            element_id: LayerData::encode_u16(&self.element_id, LayerEncoding::ZstdBase64),
            fluid_id: None,
        }
    }
}

fn air(id: u8) -> Fluid {
    Fluid {
        id,
        name: "Air".into(),
        preset: Some("air".into()),
        density: 1.204,
        kinematic_viscosity: 1.516e-5,
        thermal_conductivity: Some(0.0257),
        specific_heat: Some(1005.0),
        thermal_expansion: Some(3.43e-3),
    }
}

fn no_slip_wall(id: u16, name: &str, thermal: ThermalBc) -> Element {
    Element {
        id,
        name: Some(name.into()),
        kind: ElementKind::Wall {
            velocity: WallVelocity::NoSlip,
            thermal,
            material: None,
        },
    }
}

fn outlet(id: u16) -> Element {
    Element {
        id,
        name: Some("Outlet".into()),
        kind: ElementKind::Outlet {
            pressure: OutletBc::Pressure { value: 0.0 },
        },
    }
}

fn isothermal() -> Physics {
    Physics {
        gravity: [0.0, 0.0],
        thermal: false,
        buoyancy: Buoyancy::None,
        free_surface: false,
    }
}

fn run(end_time: f64, output_interval: f64, output_fields: Vec<OutputField>) -> RunConfig {
    RunConfig {
        method: Method::Lbm,
        backend: BackendPreference::Auto,
        precision: Precision::F32,
        end_time,
        output_interval,
        output_fields,
    }
}

/// Straight channel with a uniform inlet: the simplest possible scene.
pub fn channel() -> Scene {
    let (w, h) = (400, 100);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h - 1, w, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 2);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, 3);
    Scene {
        schema_version: SCHEMA_VERSION,
        name: "Channel".into(),
        description: Some("Air entering a 0.4 m × 0.1 m channel at 0.05 m/s (Re ≈ 330).".into()),
        grid: Grid {
            width: w,
            height: h,
            cell_size: 0.001,
            edges: DomainEdges::default(),
        },
        fluids: vec![air(1)],
        solid_materials: vec![],
        elements: vec![
            no_slip_wall(1, "Walls", ThermalBc::Adiabatic),
            Element {
                id: 2,
                name: Some("Inlet".into()),
                kind: ElementKind::Inlet {
                    velocity: InletVelocity::Uniform { value: [0.05, 0.0] },
                    thermal: ThermalBc::Adiabatic,
                    fluid: 1,
                },
            },
            outlet(3),
        ],
        probes: vec![Probe {
            name: "Centre".into(),
            position: [200, 50],
        }],
        initial: InitialConditions {
            fluid: 1,
            velocity: [0.0, 0.0],
            temperature: 293.15,
            pressure: 0.0,
        },
        physics: isothermal(),
        run: run(
            20.0,
            0.1,
            vec![
                OutputField::Velocity,
                OutputField::Pressure,
                OutputField::Vorticity,
            ],
        ),
        layers: c.layers(),
    }
}

/// Schäfer & Turek (1996) benchmark 2D-2: cylinder in a channel at Re = 100.
/// D = 0.1 m resolved with 20 cells; reference St ≈ 0.30, max C_D ≈ 3.23.
pub fn cylinder_re100() -> Scene {
    let dx = 0.005;
    let (fluid_w, fluid_h) = (440, 82); // 2.2 m × 0.41 m
    let (w, h) = (fluid_w + 2, fluid_h + 2);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h - 1, w, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 2);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, 3);
    // Centre at (0.2 m, 0.2 m) from the bottom-left corner of the fluid region.
    c.disk(
        1.0 + 0.2 / dx,
        1.0 + 0.2 / dx,
        0.05 / dx,
        CellType::Solid,
        4,
    );
    Scene {
        schema_version: SCHEMA_VERSION,
        name: "Cylinder Re=100".into(),
        description: Some(
            "Schäfer & Turek (1996) benchmark 2D-2: von Kármán vortex street behind a cylinder."
                .into(),
        ),
        grid: Grid {
            width: w,
            height: h,
            cell_size: dx,
            edges: DomainEdges::default(),
        },
        fluids: vec![Fluid {
            id: 1,
            name: "Benchmark fluid".into(),
            preset: None,
            density: 1.0,
            kinematic_viscosity: 1.0e-3,
            thermal_conductivity: None,
            specific_heat: None,
            thermal_expansion: None,
        }],
        solid_materials: vec![],
        elements: vec![
            no_slip_wall(1, "Channel walls", ThermalBc::Adiabatic),
            Element {
                id: 2,
                name: Some("Inlet".into()),
                kind: ElementKind::Inlet {
                    velocity: InletVelocity::Parabolic { peak: [1.5, 0.0] },
                    thermal: ThermalBc::Adiabatic,
                    fluid: 1,
                },
            },
            outlet(3),
            no_slip_wall(4, "Cylinder", ThermalBc::Adiabatic),
        ],
        probes: vec![
            Probe {
                name: "Front".into(),
                position: [30, 41],
            },
            Probe {
                name: "Wake".into(),
                position: [91, 41],
            },
        ],
        initial: InitialConditions {
            fluid: 1,
            velocity: [0.0, 0.0],
            temperature: 293.15,
            pressure: 0.0,
        },
        physics: isothermal(),
        run: run(
            8.0,
            0.01,
            vec![
                OutputField::Velocity,
                OutputField::Pressure,
                OutputField::Vorticity,
            ],
        ),
        layers: c.layers(),
    }
}

/// de Vahl Davis (1983) differentially heated square cavity at Ra = 1e5 (air, Pr ≈ 0.71).
/// Reference mean Nusselt number ≈ 4.519.
pub fn heated_cavity_ra1e5() -> Scene {
    let n = 128;
    let side = 0.1;
    let dx = side / n as f64;
    let fluid = air(1);
    // Ra = g β ΔT L³ / (ν α)  ⇒  ΔT
    let alpha = 0.0257 / (fluid.density * 1005.0);
    let delta_t =
        1.0e5 * fluid.kinematic_viscosity * alpha / (STANDARD_GRAVITY * 3.43e-3 * side.powi(3));
    let t_ref = 293.15;
    let round = |t: f64| (t * 1e4).round() / 1e4;

    let (w, h) = (n + 2, n + 2);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h - 1, w, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Solid, 2);
    c.rect(w - 1, 1, w, h - 1, CellType::Solid, 3);
    Scene {
        schema_version: SCHEMA_VERSION,
        name: "Heated cavity Ra=1e5".into(),
        description: Some(
            "de Vahl Davis (1983): natural convection in a square cavity, hot left wall, cold right wall."
                .into(),
        ),
        grid: Grid { width: w, height: h, cell_size: dx, edges: DomainEdges::default() },
        fluids: vec![fluid],
        solid_materials: vec![],
        elements: vec![
            no_slip_wall(1, "Adiabatic walls", ThermalBc::Adiabatic),
            no_slip_wall(2, "Hot wall", ThermalBc::Fixed { value: round(t_ref + delta_t / 2.0) }),
            no_slip_wall(3, "Cold wall", ThermalBc::Fixed { value: round(t_ref - delta_t / 2.0) }),
        ],
        probes: vec![Probe { name: "Centre".into(), position: [65, 65] }],
        initial: InitialConditions { fluid: 1, velocity: [0.0, 0.0], temperature: t_ref, pressure: 0.0 },
        physics: Physics {
            gravity: [0.0, -STANDARD_GRAVITY],
            thermal: true,
            buoyancy: Buoyancy::Boussinesq,
            free_surface: false,
        },
        run: run(200.0, 1.0, vec![OutputField::Velocity, OutputField::Temperature]),
        layers: c.layers(),
    }
}

/// Martin & Moyce (1952) dam break: a water column (a = 0.1 m, height 2a) collapsing in a tank.
pub fn dam_break() -> Scene {
    let dx = 0.0025;
    let (tank_w, tank_h) = (200, 100); // 0.5 m × 0.25 m
    let (w, h) = (tank_w + 2, tank_h + 1);
    let mut c = Canvas::new(w, h, CellType::Empty);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, 1, 1, h, CellType::Solid, 1);
    c.rect(w - 1, 1, w, h, CellType::Solid, 1);
    c.rect(1, 1, 1 + 40, 1 + 80, CellType::Fluid, 0);
    Scene {
        schema_version: SCHEMA_VERSION,
        name: "Dam break".into(),
        description: Some(
            "Martin & Moyce (1952): collapse of a water column; compare the front position x(t)."
                .into(),
        ),
        grid: Grid {
            width: w,
            height: h,
            cell_size: dx,
            edges: DomainEdges::default(),
        },
        fluids: vec![Fluid {
            id: 1,
            name: "Water".into(),
            preset: Some("water".into()),
            density: 998.2,
            kinematic_viscosity: 1.004e-6,
            thermal_conductivity: Some(0.598),
            specific_heat: Some(4182.0),
            thermal_expansion: Some(2.07e-4),
        }],
        solid_materials: vec![],
        elements: vec![no_slip_wall(1, "Tank", ThermalBc::Adiabatic)],
        probes: vec![Probe {
            name: "Right wall".into(),
            position: [199, 5],
        }],
        initial: InitialConditions {
            fluid: 1,
            velocity: [0.0, 0.0],
            temperature: 293.15,
            pressure: 0.0,
        },
        physics: Physics {
            gravity: [0.0, -STANDARD_GRAVITY],
            thermal: false,
            buoyancy: Buoyancy::None,
            free_surface: true,
        },
        run: run(
            1.0,
            0.01,
            vec![OutputField::Velocity, OutputField::FillFraction],
        ),
        layers: c.layers(),
    }
}
