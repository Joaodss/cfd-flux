//! Heat flux through the thermal boundaries of a scene, measured on a temperature field, and the
//! Nusselt number built from it (design proposal §7.3).
//!
//! It reads a *physical* temperature field (K, row-major from the bottom-left corner, the layout of
//! a stored frame), not lattice fields, so it is the same for every backend. The wall-gradient
//! formula is the one `cfd-verify`'s heated cavity uses; both call [`wall_gradient`].
//!
//! **Which elements count:** a scene element of kind `Wall` or `HeatSource` whose thermal
//! condition is `Fixed` (temperature) or `Flux`, in a thermal scene. For `Fixed` the heat flux is
//! measured from the fluid's temperature next to the element; for `Flux` it is the prescribed
//! value times the length (listed so a flux-driven wall has its Nusselt number too, from the wall
//! temperature it settles at, which a frame does not hold). Adiabatic elements carry no heat;
//! `Convective` is not supported by the solver yet; inlets carry heat by advection, which is not
//! conduction through a wall, so they are left out.

use crate::scene::{CellType, ElementKind, Scene, ThermalBc};

/// Temperature gradient normal to a wall, pointing into the fluid, in K per cell width.
///
/// The wall is half-way between its own cell centre and the first fluid cell centre: the
/// temperature there is `t_wall`, the first fluid cell has `t1` (at 0.5 cell widths from the wall)
/// and the second `t2` (at 1.5). The quadratic through those three points has the slope
/// `(-8 t_wall + 9 t1 - t2) / 3` at the wall. Without a second fluid cell (a wall one cell
/// thick next to the next obstacle) the slope is the one-sided difference `2 (t1 - t_wall)`.
pub fn wall_gradient(t_wall: f64, t1: f64, t2: Option<f64>) -> f64 {
    match t2 {
        Some(t2) => (-8.0 * t_wall + 9.0 * t1 - t2) / 3.0,
        None => 2.0 * (t1 - t_wall),
    }
}

/// Nusselt number `Nu = (q / length) L / (k ΔT)` of a boundary that carries `q` W per metre of
/// span over `length` metres of wall, with the reference length `L` (m), conductivity `k`
/// (W/(m·K)) and temperature difference `ΔT` (K). The order of the operations is part of the
/// contract with the viewer's TypeScript (`apps/web/src/viewer/statistics.ts`).
pub fn nusselt(
    q_w_per_m: f64,
    length_m: f64,
    reference_length: f64,
    conductivity: f64,
    delta_t: f64,
) -> f64 {
    q_w_per_m / length_m * reference_length / (conductivity * delta_t)
}

/// Heat through one element at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallHeat {
    /// W per metre of span, positive into the fluid.
    pub q_w_per_m: f64,
    /// Length of the element's fluid-facing boundary (m): the number of cell faces times dx.
    pub length_m: f64,
}

#[derive(Debug, Clone, Copy)]
enum Boundary {
    /// K
    Fixed(f64),
    /// W/m², into the fluid
    Flux(f64),
}

/// A cell face between a wall cell and a fluid cell: the fluid cell next to the wall and the one
/// behind it (when it is fluid too).
#[derive(Debug, Clone, Copy)]
struct Face {
    first: u32,
    second: Option<u32>,
}

#[derive(Debug, Clone)]
struct WallElement {
    id: u16,
    boundary: Boundary,
    faces: Vec<Face>,
}

/// The thermal boundaries of a scene, found once, ready to be measured on any number of frames.
#[derive(Debug, Clone)]
pub struct HeatWalls {
    dx: f64,
    conductivity: f64,
    elements: Vec<WallElement>,
}

impl HeatWalls {
    /// `None` when the scene is not thermal or has no counted element touching the fluid. Faces
    /// across a periodic edge are not followed.
    pub fn new(scene: &Scene) -> Option<Self> {
        if !scene.physics.thermal {
            return None;
        }
        let grid = &scene.grid;
        let (w, h) = (grid.width as usize, grid.height as usize);
        let cells = w * h;
        let cell_type: Vec<CellType> = scene
            .layers
            .cell_type
            .decode_u8(cells)
            .ok()?
            .into_iter()
            .map(|v| CellType::try_from(v).unwrap_or(CellType::Solid))
            .collect();
        let element_id = scene.layers.element_id.decode_u16(cells).ok()?;
        let conductivity = scene
            .fluids
            .iter()
            .find(|f| f.id == scene.initial.fluid)
            .and_then(|f| f.thermal_conductivity)
            .unwrap_or(0.0);

        let mut elements: Vec<WallElement> = scene
            .elements
            .iter()
            .filter_map(|e| {
                let thermal = match &e.kind {
                    ElementKind::Wall { thermal, .. } | ElementKind::HeatSource { thermal, .. } => {
                        thermal
                    }
                    ElementKind::Inlet { .. } | ElementKind::Outlet { .. } => return None,
                };
                let boundary = match *thermal {
                    ThermalBc::Fixed { value } => Boundary::Fixed(value),
                    ThermalBc::Flux { value } => Boundary::Flux(value),
                    ThermalBc::Adiabatic | ThermalBc::Convective { .. } => return None,
                };
                Some(WallElement {
                    id: e.id,
                    boundary,
                    faces: Vec::new(),
                })
            })
            .collect();
        let is_fluid = |x: isize, y: isize| {
            (0..w as isize).contains(&x)
                && (0..h as isize).contains(&y)
                && cell_type[y as usize * w + x as usize] == CellType::Fluid
        };
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let Some(el) = elements
                    .iter_mut()
                    .find(|e| e.id == element_id[i] && cell_type[i] != CellType::Fluid)
                else {
                    continue;
                };
                for (dx, dy) in [(1isize, 0isize), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if !is_fluid(nx, ny) {
                        continue;
                    }
                    let (sx, sy) = (nx + dx, ny + dy);
                    el.faces.push(Face {
                        first: (ny as usize * w + nx as usize) as u32,
                        second: is_fluid(sx, sy).then(|| (sy as usize * w + sx as usize) as u32),
                    });
                }
            }
        }
        elements.retain(|e| !e.faces.is_empty());
        (!elements.is_empty()).then_some(Self {
            dx: grid.cell_size,
            conductivity,
            elements,
        })
    }

    /// The element ids, in the order [`measure`](Self::measure) reports them.
    pub fn element_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.elements.iter().map(|e| e.id)
    }

    /// The heat of every element on a temperature field (K) of the grid's size.
    pub fn measure(&self, temperature: &[f32]) -> Vec<WallHeat> {
        self.elements
            .iter()
            .map(|e| {
                let length_m = e.faces.len() as f64 * self.dx;
                let q_w_per_m = match e.boundary {
                    Boundary::Flux(q) => q * length_m,
                    // q'' = -k dT/dn per face, times the face length dx: the cell width cancels.
                    Boundary::Fixed(t_wall) => {
                        let sum: f64 = e
                            .faces
                            .iter()
                            .map(|f| {
                                wall_gradient(
                                    t_wall,
                                    f64::from(temperature[f.first as usize]),
                                    f.second.map(|s| f64::from(temperature[s as usize])),
                                )
                            })
                            .sum();
                        -self.conductivity * sum
                    }
                };
                WallHeat {
                    q_w_per_m,
                    length_m,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples;

    #[test]
    fn the_gradient_is_exact_for_a_quadratic() {
        // T(x) = 3 + 2x + 0.5x² with x in cell widths from the wall: slope 2 at the wall.
        let t = |x: f64| 3.0 + 2.0 * x + 0.5 * x * x;
        let g = wall_gradient(t(0.0), t(0.5), Some(t(1.5)));
        assert!((g - 2.0).abs() < 1e-12, "{g}");
        // One-sided: exact for a linear profile.
        let g = wall_gradient(1.0, 2.0, None);
        assert_eq!(g, 2.0);
    }

    #[test]
    fn nusselt_of_a_conduction_profile_is_one() {
        // q = k ΔT / L over a wall of length L.
        let (k, dt, l) = (0.026, 10.0, 0.1);
        assert!((nusselt(k * dt / l * l, l, l, k, dt) - 1.0).abs() < 1e-12);
    }

    /// The heated cavity with a temperature field linear between the walls (pure conduction):
    /// Nu is 1 on both walls (opposite signs), the adiabatic walls carry nothing.
    #[test]
    fn a_linear_profile_between_the_cavity_walls_has_nusselt_one() {
        let scene = examples::heated_cavity_ra1e5();
        let walls = HeatWalls::new(&scene).expect("two isothermal walls");
        assert_eq!(walls.element_ids().collect::<Vec<_>>(), vec![2, 3]);
        let (w, h) = (scene.grid.width as usize, scene.grid.height as usize);
        let n = (w - 2) as f64;
        let (t_hot, t_cold) = match (&scene.elements[1].kind, &scene.elements[2].kind) {
            (
                ElementKind::Wall {
                    thermal: ThermalBc::Fixed { value: hot },
                    ..
                },
                ElementKind::Wall {
                    thermal: ThermalBc::Fixed { value: cold },
                    ..
                },
            ) => (*hot, *cold),
            other => panic!("unexpected elements {other:?}"),
        };
        // Wall surfaces at x = 0.5 and n + 0.5 (cell centres at integers).
        let t = |x: usize| t_hot - (t_hot - t_cold) * (x as f64 - 0.5) / n;
        let field: Vec<f32> = (0..w * h).map(|i| t(i % w) as f32).collect();
        let heat = walls.measure(&field);
        let k = scene.fluids[0].thermal_conductivity.unwrap();
        let side = n * scene.grid.cell_size;
        for (wall, sign) in heat.iter().zip([1.0, -1.0]) {
            assert!((wall.length_m - side).abs() < 1e-12);
            let nu = nusselt(wall.q_w_per_m, wall.length_m, side, k, t_hot - t_cold);
            // The frame stores K as f32 (3e-5 K near 293 K) and ΔT is ≈ 1 K here: a few 0.1 % per face,
            // the same for every face of this artificial field (random in a real one).
            assert!((nu - sign).abs() < 1e-2, "Nu = {nu}");
        }
    }

    #[test]
    fn scenes_without_thermal_walls_have_none() {
        assert!(HeatWalls::new(&examples::cylinder_re100()).is_none());
        assert!(HeatWalls::new(&examples::channel()).is_none());
    }
}
