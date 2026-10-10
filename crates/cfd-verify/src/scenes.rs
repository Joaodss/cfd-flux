//! Validation geometries as regular scenes, in physical units: every case goes through
//! `Scene → Domain`, so the unit conversion is validated together with the solver.
//!
//! Lengths are chosen so the characteristic length is 1 m (except the Schäfer-Turek channel,
//! which keeps the benchmark's dimensions); the resolution enters through `cellSize`.

use cfd_core::examples::Canvas;
use cfd_core::scene::*;
use cfd_core::units::STANDARD_GRAVITY;

/// Hot and cold wall temperatures of the heated cavity (K); ΔT = 1 K.
pub const T_HOT: f64 = 301.0;
pub const T_COLD: f64 = 300.0;

/// Schäfer & Turek (1996) channel: height, cylinder centre and diameter (m), ν (m²/s).
pub const ST_HEIGHT: f64 = 0.41;
pub const ST_LENGTH: f64 = 2.2;
pub const ST_CENTRE: [f64; 2] = [0.2, 0.2];
pub const ST_DIAMETER: f64 = 0.1;
pub const ST_NU: f64 = 1.0e-3;

/// Element ids used by the builders.
pub mod ids {
    pub const WALL: u16 = 1;
    /// Moving lid, top wall of a channel, inlet, or hot wall.
    pub const DRIVE: u16 = 2;
    /// Outlet or cold wall.
    pub const OUTLET: u16 = 3;
    pub const CYLINDER: u16 = 4;
}

fn fluid(nu: f64) -> Fluid {
    Fluid {
        id: 1,
        name: "Test fluid".into(),
        preset: None,
        density: 1.0,
        kinematic_viscosity: nu,
        thermal_conductivity: None,
        specific_heat: None,
        thermal_expansion: None,
    }
}

fn wall(id: u16, name: &str, velocity: [f64; 2], thermal: ThermalBc) -> Element {
    Element {
        id,
        name: Some(name.into()),
        kind: ElementKind::Wall {
            velocity: if velocity == [0.0; 2] {
                WallVelocity::NoSlip
            } else {
                WallVelocity::Moving { velocity }
            },
            thermal,
            material: None,
        },
    }
}

fn scene(
    name: String,
    canvas: &Canvas,
    cell_size: f64,
    edges: DomainEdges,
    fluid: Fluid,
    elements: Vec<Element>,
    run: (f64, f64, Vec<OutputField>),
) -> Scene {
    Scene {
        schema_version: SCHEMA_VERSION,
        name,
        description: None,
        grid: Grid {
            width: canvas.width,
            height: canvas.height,
            cell_size,
            edges,
        },
        fluids: vec![fluid],
        solid_materials: vec![],
        elements,
        probes: vec![],
        initial: InitialConditions {
            fluid: 1,
            velocity: [0.0; 2],
            temperature: T_COLD,
            pressure: 0.0,
        },
        physics: Physics {
            gravity: [0.0, 0.0],
            thermal: false,
            buoyancy: Buoyancy::None,
            free_surface: false,
        },
        run: RunConfig {
            method: Method::Lbm,
            backend: BackendPreference::Auto,
            precision: Precision::F32,
            end_time: run.0,
            output_interval: run.1,
            output_fields: run.2,
        },
        layers: canvas.layers(),
    }
}

fn flow_fields() -> Vec<OutputField> {
    vec![
        OutputField::Velocity,
        OutputField::Pressure,
        OutputField::Vorticity,
    ]
}

/// Fully periodic 1 m × 1 m box of `n²` cells, at rest (initial fields are set by the case).
pub fn periodic_box(n: u32, nu: f64) -> Scene {
    let c = Canvas::new(n, n, CellType::Fluid);
    let edges = DomainEdges {
        left: EdgeKind::Periodic,
        right: EdgeKind::Periodic,
        bottom: EdgeKind::Periodic,
        top: EdgeKind::Periodic,
    };
    scene(
        format!("Periodic box {n}²"),
        &c,
        1.0 / n as f64,
        edges,
        fluid(nu),
        vec![],
        (1.0, 0.1, flow_fields()),
    )
}

/// Plane channel of height 1 m resolved by `n` cells, periodic along x (4 cells wide), between a
/// bottom wall (`ids::WALL`) and a top wall (`ids::DRIVE`) moving at `top_velocity` m/s.
pub fn plane_channel(n: u32, nu: f64, top_velocity: f64) -> Scene {
    let (w, h) = (4, n + 2);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, ids::WALL);
    c.rect(0, h - 1, w, h, CellType::Solid, ids::DRIVE);
    let edges = DomainEdges {
        left: EdgeKind::Periodic,
        right: EdgeKind::Periodic,
        ..DomainEdges::default()
    };
    scene(
        format!("Plane channel {n}"),
        &c,
        1.0 / n as f64,
        edges,
        fluid(nu),
        vec![
            wall(ids::WALL, "Bottom wall", [0.0; 2], ThermalBc::Adiabatic),
            wall(
                ids::DRIVE,
                "Top wall",
                [top_velocity, 0.0],
                ThermalBc::Adiabatic,
            ),
        ],
        (100.0, 1.0, flow_fields()),
    )
}

/// Lid-driven square cavity (side 1 m, `n²` fluid cells, lid at 1 m/s) at Reynolds `re`.
/// The lid is the top row (`ids::DRIVE`); the other walls are the domain edges (element 0).
pub fn lid_driven_cavity(n: u32, re: f64) -> Scene {
    let mut c = Canvas::new(n, n + 1, CellType::Fluid);
    c.rect(0, n, n, n + 1, CellType::Solid, ids::DRIVE);
    scene(
        format!("Lid-driven cavity Re={re} ({n}²)"),
        &c,
        1.0 / n as f64,
        DomainEdges::default(),
        fluid(1.0 / re),
        vec![wall(ids::DRIVE, "Lid", [1.0, 0.0], ThermalBc::Adiabatic)],
        (100.0, 1.0, flow_fields()),
    )
}

/// Differentially heated square cavity (side 1 m, `n²` fluid cells): hot left wall
/// (`ids::DRIVE`, [`T_HOT`]), cold right wall (`ids::OUTLET`, [`T_COLD`]), adiabatic top and
/// bottom (domain edges). Properties give Rayleigh `ra`, Prandtl `pr` and a buoyancy velocity
/// `√(gβΔT L)` of 1 m/s.
pub fn heated_cavity(n: u32, ra: f64, pr: f64) -> Scene {
    let mut c = Canvas::new(n + 2, n, CellType::Fluid);
    c.rect(0, 0, 1, n, CellType::Solid, ids::DRIVE);
    c.rect(n + 1, 0, n + 2, n, CellType::Solid, ids::OUTLET);
    // Ra = gβΔT L³/(να) with gβΔT L = 1 m²/s² and L = 1 m  ⇒  να = 1/Ra, ν/α = Pr.
    let nu = (pr / ra).sqrt();
    let alpha = 1.0 / (pr * ra).sqrt();
    let mut f = fluid(nu);
    f.specific_heat = Some(1.0);
    f.thermal_conductivity = Some(alpha); // ρ = c_p = 1
    f.thermal_expansion = Some(1.0 / STANDARD_GRAVITY);
    let mut s = scene(
        format!("Heated cavity Ra={ra:e} ({n}²)"),
        &c,
        1.0 / n as f64,
        DomainEdges::default(),
        f,
        vec![
            wall(
                ids::DRIVE,
                "Hot wall",
                [0.0; 2],
                ThermalBc::Fixed { value: T_HOT },
            ),
            wall(
                ids::OUTLET,
                "Cold wall",
                [0.0; 2],
                ThermalBc::Fixed { value: T_COLD },
            ),
        ],
        (
            200.0,
            2.0,
            vec![OutputField::Velocity, OutputField::Temperature],
        ),
    );
    s.initial.temperature = 0.5 * (T_HOT + T_COLD);
    s.physics = Physics {
        gravity: [0.0, -STANDARD_GRAVITY],
        thermal: true,
        buoyancy: Buoyancy::Boussinesq,
        free_surface: false,
    };
    s
}

/// Schäfer & Turek (1996) channel with a cylinder resolved by `cells_per_diameter` cells (a
/// multiple of 10, so the 0.41 m height is a whole number of cells), parabolic inlet with peak
/// velocity `u_max` (0.3 m/s → Re 20, 1.5 m/s → Re 100) and a zero-pressure outlet.
/// Probes `front` and `back` sit next to the cylinder at the benchmark's Δp points.
pub fn schafer_turek(cells_per_diameter: u32, u_max: f64) -> Scene {
    assert!(
        cells_per_diameter >= 10 && cells_per_diameter % 10 == 0,
        "cells per diameter must be a multiple of 10"
    );
    let dx = ST_DIAMETER / cells_per_diameter as f64;
    let (fw, fh) = (
        (ST_LENGTH / dx).round() as u32,
        (ST_HEIGHT / dx).round() as u32,
    );
    let (w, h) = (fw + 2, fh + 2);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, ids::WALL);
    c.rect(0, h - 1, w, h, CellType::Solid, ids::WALL);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, ids::DRIVE);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, ids::OUTLET);
    // Fluid starts at cell 1 in both directions (cell-edge coordinates).
    c.disk(
        1.0 + ST_CENTRE[0] / dx,
        1.0 + ST_CENTRE[1] / dx,
        0.5 * ST_DIAMETER / dx,
        CellType::Solid,
        ids::CYLINDER,
    );
    let re = u_max * 2.0 / 3.0 * ST_DIAMETER / ST_NU;
    let mut s = scene(
        format!("Schäfer-Turek Re={re:.0} (D = {cells_per_diameter} cells)"),
        &c,
        dx,
        DomainEdges::default(),
        fluid(ST_NU),
        vec![
            wall(ids::WALL, "Channel walls", [0.0; 2], ThermalBc::Adiabatic),
            Element {
                id: ids::DRIVE,
                name: Some("Inlet".into()),
                kind: ElementKind::Inlet {
                    velocity: InletVelocity::Parabolic { peak: [u_max, 0.0] },
                    thermal: ThermalBc::Adiabatic,
                    fluid: 1,
                },
            },
            Element {
                id: ids::OUTLET,
                name: Some("Outlet".into()),
                kind: ElementKind::Outlet {
                    pressure: OutletBc::Pressure { value: 0.0 },
                },
            },
            wall(ids::CYLINDER, "Cylinder", [0.0; 2], ThermalBc::Adiabatic),
        ],
        (20.0, 0.05, flow_fields()),
    );
    // Δp points (0.15, 0.2) and (0.25, 0.2): the first fluid cell in front of / behind them.
    let cell = |x: f64, y: f64| [1 + (x / dx).floor() as u32, 1 + (y / dx).floor() as u32];
    let y = ST_CENTRE[1];
    let r = 0.5 * ST_DIAMETER;
    let mut front = cell(ST_CENTRE[0] - r - 0.5 * dx, y);
    let mut back = cell(ST_CENTRE[0] + r + 0.5 * dx, y);
    let is_fluid = |p: [u32; 2]| c.cell_type[(p[1] * w + p[0]) as usize] == CellType::Fluid as u8;
    while !is_fluid(front) {
        front[0] -= 1;
    }
    while !is_fluid(back) {
        back[0] += 1;
    }
    s.probes = vec![
        Probe {
            name: "front".into(),
            position: front,
        },
        Probe {
            name: "back".into(),
            position: back,
        },
    ];
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfd_core::{Domain, DomainOptions};

    fn domain(s: &Scene) -> Domain {
        Domain::from_scene(s, &DomainOptions::default())
            .unwrap_or_else(|r| panic!("{:#?}", r.issues))
    }

    #[test]
    fn builders_give_the_intended_dimensionless_numbers() {
        let d = domain(&lid_driven_cavity(64, 400.0));
        assert!((d.numbers.reynolds / 400.0 - 1.0).abs() < 1e-12);
        assert!((d.numbers.length - 1.0).abs() < 1e-12);

        let d = domain(&heated_cavity(64, 1e5, 0.71));
        assert!((d.numbers.rayleigh.unwrap() / 1e5 - 1.0).abs() < 1e-9);
        assert!((d.numbers.prandtl.unwrap() / 0.71 - 1.0).abs() < 1e-9);
        assert!((d.numbers.velocity - 1.0).abs() < 1e-9);

        let s = schafer_turek(20, 0.3);
        assert_eq!((s.grid.width, s.grid.height), (442, 84));
        let d = domain(&s);
        assert!(d.issues.is_empty(), "{:?}", d.issues);
        assert_eq!(d.probes[0].y, d.probes[1].y);
        assert!(d.probes[0].x < 41 && d.probes[1].x > 41);

        let d = domain(&plane_channel(16, 0.1, 0.0));
        assert_eq!(d.periodic, [true, false]);
        let d = domain(&periodic_box(16, 0.1));
        assert_eq!(d.periodic, [true, true]);
    }
}
