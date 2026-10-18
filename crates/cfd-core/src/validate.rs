//! Semantic validation of a [`Scene`]: the rules a JSON Schema cannot express
//! (layer sizes, id references, cell types matching element kinds, adjacency).
//!
//! Physical stability checks (Mach, relaxation time) belong to the solver's unit
//! conversion and are not done here.

use std::collections::{HashMap, HashSet};

use crate::scene::{CellType, EdgeKind, ElementKind, Scene, SCHEMA_VERSION};

/// Largest accepted domain (cells per side) in the 2D MVP.
pub const MAX_GRID_SIDE: u32 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `layer.length` (used by the UI for translations).
    pub code: &'static str,
    /// English text, also the fallback for codes a UI does not translate.
    pub message: String,
    /// The values in the message (element ids, numbers, limits), so a UI can word it in its own
    /// language (ADR-027).
    pub params: serde_json::Map<String, serde_json::Value>,
    /// The cell the issue is about, when there is one (the editor marks it).
    pub at: Option<[u32; 2]>,
}

impl Issue {
    /// Adds a value of the message.
    pub fn param(&mut self, key: &str, value: impl Into<serde_json::Value>) -> &mut Self {
        self.params.insert(key.to_string(), value.into());
        self
    }

    /// Sets the cell the issue is about.
    pub fn at(&mut self, x: usize, y: usize) -> &mut Self {
        self.at = Some([x as u32, y as u32]);
        self
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        !self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Issue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// Adds an error; returns it, to add `param`s and a cell (`at`).
    pub fn error(&mut self, code: &'static str, message: impl Into<String>) -> &mut Issue {
        self.push(Severity::Error, code, message.into())
    }

    /// Adds a warning; returns it, to add `param`s and a cell (`at`).
    pub fn warning(&mut self, code: &'static str, message: impl Into<String>) -> &mut Issue {
        self.push(Severity::Warning, code, message.into())
    }

    fn push(&mut self, severity: Severity, code: &'static str, message: String) -> &mut Issue {
        self.issues.push(Issue {
            severity,
            code,
            message,
            params: serde_json::Map::new(),
            at: None,
        });
        self.issues.last_mut().expect("just pushed")
    }
}

/// At most this many per-cell errors of the same kind are reported.
const MAX_CELL_ERRORS: usize = 5;

pub fn validate(scene: &Scene) -> Report {
    let mut r = Report::default();
    let grid = &scene.grid;

    if scene.schema_version != SCHEMA_VERSION {
        r.error(
            "scene.version",
            format!(
                "schemaVersion {} is not supported (expected {SCHEMA_VERSION})",
                scene.schema_version
            ),
        )
        .param("version", scene.schema_version)
        .param("expected", SCHEMA_VERSION);
    }
    if grid.width == 0 || grid.height == 0 {
        r.error("grid.empty", "grid width and height must be at least 1");
        return r;
    }
    if grid.width > MAX_GRID_SIDE || grid.height > MAX_GRID_SIDE {
        r.error(
            "grid.tooLarge",
            format!("grid sides must be at most {MAX_GRID_SIDE} cells"),
        )
        .param("max", MAX_GRID_SIDE);
        return r;
    }
    if !(grid.cell_size.is_finite() && grid.cell_size > 0.0) {
        r.error(
            "grid.cellSize",
            "cellSize must be a positive number of metres",
        );
    }
    let e = &grid.edges;
    if (e.left == EdgeKind::Periodic) != (e.right == EdgeKind::Periodic)
        || (e.bottom == EdgeKind::Periodic) != (e.top == EdgeKind::Periodic)
    {
        r.error("grid.edges", "periodic edges must come in opposite pairs");
    }

    // Materials and fluids.
    let fluid_ids = unique_ids(&mut r, "fluid", scene.fluids.iter().map(|f| f.id as u32));
    let solid_ids = unique_ids(
        &mut r,
        "solidMaterial",
        scene.solid_materials.iter().map(|m| m.id as u32),
    );
    for f in &scene.fluids {
        if !(f.density > 0.0 && f.kinematic_viscosity > 0.0) {
            r.error(
                "fluid.properties",
                format!("fluid {} needs positive density and viscosity", f.id),
            )
            .param("fluid", f.id);
        }
        if scene.physics.thermal && (f.thermal_conductivity.is_none() || f.specific_heat.is_none())
        {
            r.error(
                "fluid.thermal",
                format!(
                    "fluid {} needs thermalConductivity and specificHeat when thermal is on",
                    f.id
                ),
            )
            .param("fluid", f.id);
        }
        if scene.physics.buoyancy == crate::scene::Buoyancy::Boussinesq
            && f.thermal_expansion.is_none()
        {
            r.error(
                "fluid.expansion",
                format!("fluid {} needs thermalExpansion for Boussinesq", f.id),
            )
            .param("fluid", f.id);
        }
    }
    if !fluid_ids.contains(&(scene.initial.fluid as u32)) {
        r.error(
            "initial.fluid",
            format!("initial fluid {} does not exist", scene.initial.fluid),
        )
        .param("fluid", scene.initial.fluid);
    }

    // Elements.
    unique_ids(
        &mut r,
        "element",
        scene.elements.iter().map(|e| e.id as u32),
    );
    let mut kinds: HashMap<u16, CellType> = HashMap::new();
    for el in &scene.elements {
        kinds.insert(el.id, el.kind.cell_type());
        let (fluid, material) = match &el.kind {
            ElementKind::Inlet { fluid, .. } => (Some(*fluid), None),
            ElementKind::Wall { material, .. } | ElementKind::HeatSource { material, .. } => {
                (None, *material)
            }
            ElementKind::Outlet { .. } => (None, None),
        };
        if let Some(f) = fluid.filter(|f| !fluid_ids.contains(&(*f as u32))) {
            r.error(
                "element.fluid",
                format!("element {} references missing fluid {f}", el.id),
            )
            .param("element", el.id)
            .param("fluid", f);
        }
        if let Some(m) = material.filter(|m| !solid_ids.contains(&(*m as u32))) {
            r.error(
                "element.material",
                format!("element {} references missing material {m}", el.id),
            )
            .param("element", el.id)
            .param("material", m);
        }
    }

    // Run settings.
    let run = &scene.run;
    if !(run.end_time > 0.0 && run.output_interval > 0.0 && run.output_interval <= run.end_time) {
        r.error("run.time", "need 0 < outputInterval <= endTime");
    }

    // Layers.
    let cells = grid.cell_count();
    let cell_type = match scene.layers.cell_type.decode_u8(cells) {
        Ok(v) => v,
        Err(e) => {
            r.error("layer.cellType", format!("cellType layer: {e}"));
            return r;
        }
    };
    let element_id = match scene.layers.element_id.decode_u16(cells) {
        Ok(v) => v,
        Err(e) => {
            r.error("layer.elementId", format!("elementId layer: {e}"));
            return r;
        }
    };
    if let Some(layer) = &scene.layers.fluid_id {
        match layer.decode_u8(cells) {
            Ok(v) => {
                let missing: HashSet<u8> = v
                    .into_iter()
                    .filter(|&f| f != 0 && !fluid_ids.contains(&(f as u32)))
                    .collect();
                if !missing.is_empty() {
                    r.error(
                        "layer.fluidId",
                        format!("fluidId layer uses missing fluids {missing:?}"),
                    );
                }
            }
            Err(e) => {
                r.error("layer.fluidId", format!("fluidId layer: {e}"));
            }
        }
    }

    let w = grid.width as usize;
    let h = grid.height as usize;
    let mut cell_errors = 0usize;
    let mut has_fluid = false;
    let mut has_inlet = false;
    let mut has_outlet = false;
    for idx in 0..cells {
        let (x, y) = (idx % w, idx / w);
        let ct = match CellType::try_from(cell_type[idx]) {
            Ok(ct) => ct,
            Err(v) => {
                if cell_errors < MAX_CELL_ERRORS {
                    r.error(
                        "cell.type",
                        format!("cell ({x}, {y}) has unknown cellType {v}"),
                    )
                    .at(x, y)
                    .param("value", v);
                }
                cell_errors += 1;
                continue;
            }
        };
        let eid = element_id[idx];
        match ct {
            CellType::Fluid => has_fluid = true,
            CellType::Empty if !scene.physics.free_surface => {
                if cell_errors < MAX_CELL_ERRORS {
                    r.error(
                        "cell.empty",
                        format!("cell ({x}, {y}) is empty but freeSurface is off"),
                    )
                    .at(x, y);
                }
                cell_errors += 1;
            }
            _ => {}
        }
        has_inlet |= ct == CellType::Inlet;
        has_outlet |= ct == CellType::Outlet;

        let needs_element = matches!(
            ct,
            CellType::Solid | CellType::Inlet | CellType::Outlet | CellType::HeatSource
        );
        // (the `problem` param for a UI, the English message)
        let problem = match (needs_element, eid) {
            (true, 0) => Some((
                "noElement",
                format!("cell ({x}, {y}) of type {ct:?} has no element"),
            )),
            (true, id) => match kinds.get(&id) {
                None => Some((
                    "missingElement",
                    format!("cell ({x}, {y}) references missing element {id}"),
                )),
                Some(k) if *k != ct => Some((
                    "kindMismatch",
                    format!("cell ({x}, {y}) is {ct:?} but element {id} is {k:?}"),
                )),
                Some(_)
                    if matches!(ct, CellType::Inlet | CellType::Outlet)
                        && !touches_fluid(&cell_type, w, h, x, y) =>
                {
                    Some((
                        "noFluidNeighbour",
                        format!("{ct:?} cell ({x}, {y}) does not touch any fluid cell"),
                    ))
                }
                Some(_) => None,
            },
            (false, 0) => None,
            (false, id) => Some((
                "unexpectedElement",
                format!("cell ({x}, {y}) of type {ct:?} must not have element {id}"),
            )),
        };
        if let Some((kind, msg)) = problem {
            if cell_errors < MAX_CELL_ERRORS {
                let issue = r.error("cell.element", msg).at(x, y).param("problem", kind);
                if eid != 0 {
                    issue.param("element", eid);
                }
            }
            cell_errors += 1;
        }
    }
    if cell_errors > MAX_CELL_ERRORS {
        r.error(
            "cell.more",
            format!("{} more cell errors omitted", cell_errors - MAX_CELL_ERRORS),
        )
        .param("count", cell_errors - MAX_CELL_ERRORS);
    }

    if !has_fluid {
        r.error("domain.noFluid", "the domain has no fluid cells");
    }
    if has_inlet && !has_outlet {
        r.warning(
            "domain.noOutlet",
            "there is an inlet but no outlet: mass will accumulate",
        );
    }
    for p in &scene.probes {
        let [px, py] = p.position;
        if px >= grid.width || py >= grid.height {
            r.error(
                "probe.position",
                format!("probe '{}' is outside the grid", p.name),
            )
            .param("probe", p.name.clone());
        }
    }
    r
}

fn touches_fluid(cell_type: &[u8], w: usize, h: usize, x: usize, y: usize) -> bool {
    let fluid = CellType::Fluid as u8;
    let mut neighbours = Vec::with_capacity(4);
    if x > 0 {
        neighbours.push((x - 1, y));
    }
    if x + 1 < w {
        neighbours.push((x + 1, y));
    }
    if y > 0 {
        neighbours.push((x, y - 1));
    }
    if y + 1 < h {
        neighbours.push((x, y + 1));
    }
    neighbours
        .into_iter()
        .any(|(nx, ny)| cell_type[ny * w + nx] == fluid)
}

fn unique_ids(r: &mut Report, what: &str, ids: impl Iterator<Item = u32>) -> HashSet<u32> {
    let mut seen = HashSet::new();
    for id in ids {
        if id == 0 {
            r.error("id.zero", format!("{what} ids must be >= 1"))
                .param("what", what);
        } else if !seen.insert(id) {
            r.error("id.duplicate", format!("duplicate {what} id {id}"))
                .param("what", what)
                .param("id", id);
        }
    }
    seen
}
