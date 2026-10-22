//! REST handlers. Each is an `async fn` whose arguments are *extractors* (`State`, `Path`,
//! `Query`, `ApiJson`): axum builds them from the request before calling it, as ASP.NET's model
//! binding does for controller actions.

use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use cfd_core::scene::BackendPreference;
use cfd_core::Scene;
use cfd_engines::{bytes_per_cell, ChooseError, EngineInfo, EngineKind};
use cfd_io::frame::{Dtype, Frame};
use cfd_job::{PrepareError, Prepared, RunOptions};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::config::{Limits, StreamDefaults};
use crate::error::{ApiError, ApiJson, IssueDto};
use crate::jobs::{JobStatus, JobView};
use crate::stream::StreamFormat;
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
    /// Characteristic scales, dimensionless numbers and lattice parameters of the run.
    pub numbers: Numbers,
}

/// What `Scene → Domain` derived (`cfd_core::domain::Dimensionless`, units, relaxation times).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Numbers {
    /// Characteristic velocity U (m/s) and length L (m: the shorter side of the fluid's box).
    pub velocity: f64,
    pub length: f64,
    pub reynolds: f64,
    /// Lattice Mach number of U.
    pub mach: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prandtl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rayleigh: Option<f64>,
    /// Viscous relaxation time τ (and the thermal one when temperature is solved).
    pub tau: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tau_thermal: Option<f64>,
    /// Cell size (m) and time step (s).
    pub dx: f64,
    pub dt: f64,
    /// Lattice velocity U·dt/dx.
    pub lattice_velocity: f64,
    /// The scales of reported numbers (the scene's `reference`, defaults filled in) and the
    /// Reynolds and Rayleigh numbers they give.
    pub reference: ReferenceNumbers,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceNumbers {
    pub length: f64,
    pub velocity: f64,
    pub temperature_difference: f64,
    pub reynolds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rayleigh: Option<f64>,
}

impl Numbers {
    fn of(d: &cfd_core::Domain) -> Self {
        let n = &d.numbers;
        Self {
            velocity: n.velocity,
            length: n.length,
            reynolds: n.reynolds,
            mach: n.mach,
            prandtl: n.prandtl,
            rayleigh: n.rayleigh,
            tau: d.physics.tau(),
            tau_thermal: d.physics.alpha.map(cfd_core::units::relaxation_time),
            dx: d.units.dx,
            dt: d.units.dt,
            lattice_velocity: n.velocity * d.units.velocity_factor(),
            reference: ReferenceNumbers {
                length: n.reference.length,
                velocity: n.reference.velocity,
                temperature_difference: n.reference.temperature_difference,
                reynolds: n.reference.reynolds,
                rayleigh: n.reference.rayleigh,
            },
        }
    }
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
    check_with_estimate(state, scene, options).map_err(|(e, _)| e)
}

/// As [`check`], and when the scene could be prepared but a limit or the engine choice failed,
/// the estimate too (the editor shows why: "17,000 steps over the limit").
fn check_with_estimate(
    state: &AppState,
    scene: &RawValue,
    options: &JobOptions,
) -> Result<Checked, (ApiError, Option<Box<Estimate>>)> {
    let s = &state.inner;
    let prepared = match cfd_job::prepare(scene.get().to_string(), options.run_options()) {
        Ok(p) => p,
        Err(PrepareError::Json(e)) => {
            return Err((
                ApiError::BadRequest {
                    code: "scene.format",
                    message: format!("invalid scene: {e}"),
                },
                None,
            ))
        }
        Err(PrepareError::Invalid(report)) => {
            return Err((
                ApiError::Unprocessable {
                    code: "scene.invalid",
                    message: "the scene cannot be run".into(),
                    issues: report.issues.iter().map(IssueDto::from).collect(),
                },
                None,
            ))
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
    let fluid_cells = d.fluid_cell_count() as u64;
    let mut estimate = Estimate {
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
        engine: None,
        seconds: None,
        numbers: Numbers::of(d),
    };
    let limits = &s.config.limits;
    if cells > limits.max_cells {
        return Err((
            unprocessable(
                "limits.cells",
                format!(
                    "{cells} cells exceed the server's limit of {}",
                    limits.max_cells
                ),
            ),
            Some(Box::new(estimate)),
        ));
    }
    if steps > limits.max_steps {
        return Err((
            unprocessable(
                "limits.steps",
                format!(
                    "{steps} steps exceed the server's limit of {}; shorten the end time or raise \
                     the lattice velocity",
                    limits.max_steps
                ),
            ),
            Some(Box::new(estimate)),
        ));
    }
    let pref = options.backend.unwrap_or(prepared.scene.run.backend);
    let mut notes = Vec::new();
    let engine = match s.inventory.choose(pref, cells, thermal, &mut notes) {
        Ok(engine) => engine,
        Err(e) => {
            let err = match e {
                ChooseError::Unavailable { .. } => {
                    unprocessable("engine.unavailable", e.to_string())
                }
                ChooseError::TooLarge(m) => unprocessable("engine.memory", m),
            };
            return Err((err, Some(Box::new(estimate))));
        }
    };
    estimate.engine = Some(engine);
    estimate.seconds = s
        .mlups(engine)
        .map(|mlups| fluid_cells as f64 * steps as f64 / (mlups * 1e6));
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
    Ok(Json(
        match check_with_estimate(&state, &scene, &JobOptions::default()) {
            Ok(c) => Validation {
                ok: true,
                code: None,
                message: None,
                issues: c.warnings,
                estimate: Some(c.estimate),
            },
            Err((
                ApiError::Unprocessable {
                    code,
                    message,
                    issues,
                },
                estimate,
            )) => Validation {
                ok: false,
                code: Some(code),
                message: Some(message),
                issues,
                estimate: estimate.map(|e| *e),
            },
            Err((e, _)) => return Err(e),
        },
    ))
}

// ---- jobs --------------------------------------------------------------------------------

/// `POST /api/simulations` body: a scene (or the id of a saved one) and run options.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitRequest {
    /// Kept exactly as sent: it is stored as the job's `scene.json`.
    scene: Option<Box<RawValue>>,
    scene_id: Option<String>,
    #[serde(default)]
    options: JobOptions,
}

/// `POST /api/simulations` query.
#[derive(Deserialize)]
pub struct SubmitQuery {
    /// `completed`: if a job of the same scene and options has completed, answer with it (`200`)
    /// instead of running again ("Open with results" in the gallery).
    reuse: Option<String>,
}

/// SHA-256 (hex) of what decides a run's results: the validated scene's canonical JSON (the
/// serialisation of the parsed `Scene`, so formatting and key order of the request do not matter)
/// and the run options that change the output. The engine is left out: the backends agree to
/// within their parity tolerances, and "which engine ran it" stays visible on the job.
pub fn run_hash(prepared: &Prepared, options: &JobOptions) -> Result<String, ApiError> {
    let scene = serde_json::to_string(&prepared.scene).map_err(anyhow::Error::from)?;
    let options = serde_json::to_string(&JobOptions {
        backend: None,
        ..options.clone()
    })
    .map_err(anyhow::Error::from)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(format!("{scene}\n{options}"))
    ))
}

/// `POST /api/simulations`: validates and queues a job; `201` with the job, or `422` with the
/// issues that prevent running it. With `?reuse=completed`, `200` with the latest completed job
/// of the same scene and options if there is one.
pub async fn submit(
    State(state): State<AppState>,
    Query(q): Query<SubmitQuery>,
    ApiJson(req): ApiJson<SubmitRequest>,
) -> Result<(StatusCode, [(HeaderName, String); 1], Json<JobView>), ApiError> {
    let reuse = match q.reuse.as_deref() {
        None => false,
        Some("completed") => true,
        Some(other) => {
            return Err(ApiError::BadRequest {
                code: "request.reuse",
                message: format!("unknown reuse {other:?}: use `completed`"),
            })
        }
    };
    let db = &state.inner.jobs.db;
    let scene = match (req.scene, &req.scene_id) {
        (Some(scene), _) => scene,
        (None, Some(id)) => {
            let json = db
                .scene_json(id)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("no saved scene {id}")))?;
            RawValue::from_string(json).map_err(anyhow::Error::from)?
        }
        (None, None) => {
            return Err(ApiError::BadRequest {
                code: "request.scene",
                message: "give a `scene` or a `sceneId`".into(),
            })
        }
    };
    if let Some(id) = &req.scene_id {
        if db.scene_json(id).await?.is_none() {
            return Err(ApiError::NotFound(format!("no saved scene {id}")));
        }
    }
    // Decoding the layers and building the domain is CPU work (≈ 0.1 s for 4096²): it runs on
    // tokio's blocking pool, not on the threads that serve requests (`Task.Run` in C#).
    let (st, options) = (state.clone(), req.options.clone());
    let checked = tokio::task::spawn_blocking(move || check(&st, &scene, &options))
        .await
        .map_err(anyhow::Error::from)??;
    let hash = run_hash(&checked.prepared, &req.options)?;
    if reuse {
        if let Some(view) = state.inner.jobs.find_reusable(&hash).await? {
            let location = format!("/api/simulations/{}", view.id);
            return Ok((StatusCode::OK, [(LOCATION, location)], Json(view)));
        }
    }
    let view = state
        .inner
        .jobs
        .submit(&state, checked, &req.options, req.scene_id, hash)
        .await?;
    let location = format!("/api/simulations/{}", view.id);
    Ok((StatusCode::CREATED, [(LOCATION, location)], Json(view)))
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    limit: Option<u32>,
}

/// `GET /api/simulations?status=&limit=`: newest first.
pub async fn list_jobs(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<JobView>>, ApiError> {
    if let Some(s) = &q.status {
        if JobStatus::parse(s).is_none() {
            return Err(ApiError::BadRequest {
                code: "request.status",
                message: format!("unknown status {s:?}"),
            });
        }
    }
    let jobs = &state.inner.jobs;
    let rows = jobs
        .db
        .jobs(q.status.as_deref(), q.limit.unwrap_or(100).min(1000))
        .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| match jobs.get(&r.id) {
                Some(live) => live.view(),
                None => JobView::from_row(r),
            })
            .collect(),
    ))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobView>, ApiError> {
    Ok(Json(state.inner.jobs.view(&id).await?))
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    purge: bool,
}

/// `DELETE /api/simulations/{id}`: cancels an active job (`202`, applied between batches).
/// With `?purge=true` also deletes its results and record (`204` once done; `202` while an
/// active job is still stopping, after which it is purged).
pub async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> Result<Response, ApiError> {
    let jobs = &state.inner.jobs;
    if q.purge {
        return Ok(if jobs.purge(&id).await? {
            StatusCode::NO_CONTENT.into_response()
        } else {
            (StatusCode::ACCEPTED, Json(jobs.view(&id).await?)).into_response()
        });
    }
    let view = jobs.view(&id).await?;
    if !view.status.is_active() {
        return Err(ApiError::Conflict(format!(
            "the job is already {}",
            view.status.as_str()
        )));
    }
    Ok((StatusCode::ACCEPTED, Json(jobs.cancel(&id).await?)).into_response())
}

/// `POST /api/simulations/{id}/pause`: `202`; the job pauses after its current batch.
pub async fn pause(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    Ok((
        StatusCode::ACCEPTED,
        Json(state.inner.jobs.pause(&id, true)?),
    ))
}

pub async fn resume(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    Ok((
        StatusCode::ACCEPTED,
        Json(state.inner.jobs.pause(&id, false)?),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameQuery {
    dtype: Option<String>,
    /// Field names, comma-separated (default: every field of the frame).
    fields: Option<String>,
    /// Longest side in cells: box-downsampled like the stream (default: full resolution).
    max_size: Option<u32>,
}

/// `GET /api/simulations/{id}/frames/{n}?dtype=f32|f16&fields=…&maxSize=…`: one stored frame
/// (LFFR). With `fields` and `maxSize` it is in a stream format: the named fields,
/// box-downsampled by the stream's integer factor with the CPU filter the GPU samples are tested
/// against, so it equals the stream frame of the same index (the viewer mixes both).
pub async fn frame(
    State(state): State<AppState>,
    Path((id, n)): Path<(String, u32)>,
    Query(q): Query<FrameQuery>,
) -> Result<([(HeaderName, &'static str); 1], Vec<u8>), ApiError> {
    let dir = state
        .inner
        .jobs
        .store
        .open(&id)
        .ok_or_else(|| ApiError::NotFound(format!("no results for job {id}")))?;
    let bytes = tokio::fs::read(dir.frame_path(n))
        .await
        .map_err(|_| ApiError::NotFound(format!("job {id} has no stored frame {n}")))?;
    let dtype = match q.dtype.as_deref() {
        None | Some("f32") => Dtype::F32,
        Some("f16") => Dtype::F16,
        Some(other) => {
            return Err(ApiError::BadRequest {
                code: "request.dtype",
                message: format!("unknown dtype {other:?} (f32 or f16)"),
            })
        }
    };
    if dtype == Dtype::F32 && q.fields.is_none() && q.max_size.is_none() {
        return Ok(([(CONTENT_TYPE, "application/octet-stream")], bytes));
    }
    let format = StreamFormat {
        fields: q.fields.as_deref().map(crate::stream::field_list),
        max_size: q.max_size.unwrap_or(u32::MAX).clamp(16, u32::MAX),
    };
    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let frame = Frame::decode(&bytes)?;
        Ok(convert_frame(frame, &format, dtype).encode()?)
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(([(CONTENT_TYPE, "application/octet-stream")], bytes))
}

/// A stored frame in a stream format: its fields in frame order, downsampled, in `dtype`.
fn convert_frame(mut frame: Frame, format: &StreamFormat, dtype: Dtype) -> Frame {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let k = format.factor(frame.width, frame.height) as usize;
    frame.fields.retain(|f| {
        format
            .fields
            .as_ref()
            .is_none_or(|list| list.contains(&f.desc.name))
    });
    for f in &mut frame.fields {
        f.values = cfd_core::sample::downsample(&f.values, f.desc.components as usize, w, h, k);
        f.desc.dtype = dtype;
    }
    frame.width = w.div_ceil(k) as u32;
    frame.height = h.div_ceil(k) as u32;
    frame
}

/// `GET /api/simulations/{id}/files/{name}`: `scene.json`, `meta.json` or a CSV series (`heat.csv`
/// only for thermal runs with stored frames).
pub async fn file(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> Result<([(HeaderName, &'static str); 1], Vec<u8>), ApiError> {
    let content_type = match name.as_str() {
        "scene.json" | "meta.json" => "application/json",
        "probes.csv" | "forces.csv" | "diagnostics.csv" | "heat.csv" => "text/csv; charset=utf-8",
        _ => return Err(ApiError::NotFound(format!("no file {name:?}"))),
    };
    let dir = state
        .inner
        .jobs
        .store
        .open(&id)
        .ok_or_else(|| ApiError::NotFound(format!("no results for job {id}")))?;
    let bytes = tokio::fs::read(dir.path().join(&name))
        .await
        .map_err(|_| ApiError::NotFound(format!("job {id} has no {name}")))?;
    Ok(([(CONTENT_TYPE, content_type)], bytes))
}

// ---- saved scenes ------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedScene {
    id: String,
    name: String,
}

/// `POST /api/scenes`: saves a scene (it must parse; it need not be runnable).
pub async fn save_scene(
    State(state): State<AppState>,
    ApiJson(scene): ApiJson<Box<RawValue>>,
) -> Result<(StatusCode, Json<SavedScene>), ApiError> {
    let parsed = Scene::from_json(scene.get()).map_err(|e| ApiError::BadRequest {
        code: "scene.format",
        message: format!("invalid scene: {e}"),
    })?;
    let id = uuid::Uuid::now_v7().to_string();
    state
        .inner
        .jobs
        .db
        .insert_scene(&id, &parsed.name, scene.get())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(SavedScene {
            id,
            name: parsed.name,
        }),
    ))
}

/// `PUT /api/scenes/{id}`: replaces a saved scene (the editor's "Save" after "Save to server").
pub async fn update_scene(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ApiJson(scene): ApiJson<Box<RawValue>>,
) -> Result<Json<SavedScene>, ApiError> {
    let parsed = Scene::from_json(scene.get()).map_err(|e| ApiError::BadRequest {
        code: "scene.format",
        message: format!("invalid scene: {e}"),
    })?;
    if !state
        .inner
        .jobs
        .db
        .update_scene(&id, &parsed.name, scene.get())
        .await?
    {
        return Err(ApiError::NotFound(format!("no saved scene {id}")));
    }
    Ok(Json(SavedScene {
        id,
        name: parsed.name,
    }))
}

pub async fn list_scenes(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::db::SceneSummary>>, ApiError> {
    Ok(Json(state.inner.jobs.db.scenes().await?))
}

pub async fn get_scene(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<([(HeaderName, &'static str); 1], String), ApiError> {
    let json = state
        .inner
        .jobs
        .db
        .scene_json(&id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("no saved scene {id}")))?;
    Ok(([(CONTENT_TYPE, "application/json")], json))
}

pub async fn delete_scene(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if state.inner.jobs.db.delete_scene(&id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("no saved scene {id}")))
    }
}

/// Any route that does not exist.
pub async fn not_found() -> ApiError {
    ApiError::NotFound("no such route".into())
}
