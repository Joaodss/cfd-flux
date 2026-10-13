//! REST handlers. Each is an `async fn` whose arguments are *extractors* (`State`, `Path`,
//! `Query`, `ApiJson`): axum builds them from the request before calling it, as ASP.NET's model
//! binding does for controller actions.

use axum::extract::State;
use axum::Json;
use cfd_core::scene::BackendPreference;
use cfd_engines::{bytes_per_cell, ChooseError, EngineInfo, EngineKind};
use cfd_job::{PrepareError, Prepared, RunOptions};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::config::{Limits, StreamDefaults};
use crate::error::{ApiError, ApiJson, IssueDto};
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    version: &'static str,
    methods: [&'static str; 1],
    engines: Vec<EngineInfo>,
    /// The engine `auto` takes (memory permitting).
    auto: Option<EngineKind>,
    limits: Limits,
    defaults: StreamDefaults,
}

pub async fn capabilities(State(state): State<AppState>) -> Json<Capabilities> {
    let s = &state.inner;
    Json(Capabilities {
        version: env!("CARGO_PKG_VERSION"),
        methods: ["lbm"],
        engines: s.inventory.engines().cloned().collect(),
        auto: s.inventory.auto(),
        limits: s.config.limits.clone(),
        defaults: s.config.stream.clone(),
    })
}

/// Overrides for one run (the same as `cfd-cli run`'s options).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobOptions {
    /// Engine; defaults to the scene's `run.backend`.
    pub backend: Option<BackendPreference>,
    pub lattice_velocity: Option<f64>,
    /// Simulated time (s), overriding `run.endTime`.
    pub end_time: Option<f64>,
    /// Time between frames (s), overriding `run.outputInterval`.
    pub output_interval: Option<f64>,
    /// Time between probe/force/diagnostic samples (s); default output interval / 10.
    pub sample_interval: Option<f64>,
    /// Store frames on disk (default true); streaming works either way.
    pub store_frames: Option<bool>,
}

impl JobOptions {
    pub fn run_options(&self) -> RunOptions {
        let d = RunOptions::default();
        RunOptions {
            lattice_velocity: self.lattice_velocity.unwrap_or(d.lattice_velocity),
            end_time: self.end_time,
            output_interval: self.output_interval,
            sample_interval: self.sample_interval,
            write_frames: self.store_frames.unwrap_or(true),
            ..d
        }
    }
}

/// What running a scene would take.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub cells: u64,
    pub fluid_cells: u64,
    pub steps: u64,
    pub frames: u64,
    pub thermal: bool,
    /// Device memory of the solver.
    pub memory_bytes: u64,
    /// Stored frames before compression (f32).
    pub stored_bytes: u64,
    /// The engine that would run it (`null` when none can).
    pub engine: Option<EngineKind>,
    /// Expected solver time, from the throughput of earlier jobs on that engine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
}

/// A scene that passed every check: ready to run on `engine`.
pub struct Checked {
    pub prepared: Prepared,
    pub estimate: Estimate,
    pub engine: EngineKind,
    pub warnings: Vec<IssueDto>,
    /// Why better engines were skipped.
    pub notes: Vec<String>,
}

fn unprocessable(code: &'static str, message: String) -> ApiError {
    ApiError::Unprocessable {
        code,
        message,
        issues: Vec::new(),
    }
}

/// Validates a scene with its options: format and consistency (`cfd_core::validate`), stability
/// (`Domain::from_scene`), the server's limits, and an engine that can hold it.
pub fn check(
    state: &AppState,
    scene: &RawValue,
    options: &JobOptions,
) -> Result<Checked, ApiError> {
    let s = &state.inner;
    let prepared = match cfd_job::prepare(scene.get().to_string(), options.run_options()) {
        Ok(p) => p,
        Err(PrepareError::Json(e)) => {
            return Err(ApiError::BadRequest {
                code: "scene.format",
                message: format!("invalid scene: {e}"),
            })
        }
        Err(PrepareError::Invalid(report)) => {
            return Err(ApiError::Unprocessable {
                code: "scene.invalid",
                message: "the scene cannot be run".into(),
                issues: report.issues.iter().map(IssueDto::from).collect(),
            })
        }
    };
    let d = &prepared.domain;
    let cells = d.cell_count() as u64;
    let thermal = d.physics.thermal();
    let steps = d.run.total;
    let frames = steps / d.run.output_every.max(1) + 1;
    let components: u64 = prepared
        .frame_fields
        .iter()
        .map(|f| match f {
            cfd_core::scene::OutputField::Velocity => 2,
            _ => 1,
        })
        .sum();
    let limits = &s.config.limits;
    if cells > limits.max_cells {
        return Err(unprocessable(
            "limits.cells",
            format!(
                "{cells} cells exceed the server's limit of {}",
                limits.max_cells
            ),
        ));
    }
    if steps > limits.max_steps {
        return Err(unprocessable(
            "limits.steps",
            format!(
                "{steps} steps exceed the server's limit of {}; shorten the end time or raise the \
                 lattice velocity",
                limits.max_steps
            ),
        ));
    }
    let pref = options.backend.unwrap_or(prepared.scene.run.backend);
    let mut notes = Vec::new();
    let engine = s
        .inventory
        .choose(pref, cells, thermal, &mut notes)
        .map_err(|e| match e {
            ChooseError::Unavailable { .. } => unprocessable("engine.unavailable", e.to_string()),
            ChooseError::TooLarge(m) => unprocessable("engine.memory", m),
        })?;
    let fluid_cells = d.fluid_cell_count() as u64;
    let estimate = Estimate {
        cells,
        fluid_cells,
        steps,
        frames,
        thermal,
        memory_bytes: cells * bytes_per_cell(thermal),
        stored_bytes: if prepared.options.write_frames {
            frames * cells * components * 4
        } else {
            0
        },
        engine: Some(engine),
        seconds: s
            .mlups(engine)
            .map(|mlups| fluid_cells as f64 * steps as f64 / (mlups * 1e6)),
    };
    let warnings = d.issues.iter().map(IssueDto::from).collect();
    Ok(Checked {
        prepared,
        estimate,
        engine,
        warnings,
        notes,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    ok: bool,
    /// The error's code when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    issues: Vec<IssueDto>,
    estimate: Option<Estimate>,
}

/// `POST /api/scenes/validate`: every check of a submission, without running anything. A scene
/// that cannot run is a normal answer here (`ok: false`), not an error status.
pub async fn validate(
    State(state): State<AppState>,
    ApiJson(scene): ApiJson<Box<RawValue>>,
) -> Result<Json<Validation>, ApiError> {
    Ok(Json(match check(&state, &scene, &JobOptions::default()) {
        Ok(c) => Validation {
            ok: true,
            code: None,
            message: None,
            issues: c.warnings,
            estimate: Some(c.estimate),
        },
        Err(ApiError::Unprocessable {
            code,
            message,
            issues,
        }) => Validation {
            ok: false,
            code: Some(code),
            message: Some(message),
            issues,
            estimate: None,
        },
        Err(e) => return Err(e),
    }))
}

/// Any route that does not exist.
pub async fn not_found() -> ApiError {
    ApiError::NotFound("no such route".into())
}
