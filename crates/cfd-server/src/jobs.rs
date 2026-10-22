//! Jobs: states, the manager, per-device job slots and the bridge between the async server and
//! the job's own threads.
//!
//! ```text
//! queued ──▶ running ⇄ paused
//!    │          │  ╲
//!    ▼          ▼   ╲──▶ failed (diverged | error | interrupted)
//! cancelled ◀───┘        completed
//! ```
//!
//! A submitted job gets a tokio task that waits for its device's semaphore (FIFO), then starts a
//! dedicated OS thread running `cfd_job::run` and hands it the permit: the permit is released when
//! the thread ends, whatever the outcome (RAII, like `using` in C#). The solver lives on that
//! thread; handlers talk to it only through messages:
//!
//! - commands (pause, resume, cancel) → a `std::sync::mpsc` channel the job's loop drains between
//!   batches (sending never blocks a handler);
//! - events (status, progress, log, done) → a `tokio::sync::broadcast` channel every viewer
//!   subscribes to;
//! - frames → the stream hub (`stream.rs`), which keeps only the latest frame per format.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use cfd_core::sample::SampleSpec;
use cfd_core::scene::OutputField;
use cfd_engines::{EngineKind, Inventory, SolverChoice};
use cfd_job::{
    Command, FrameEvent, JobContext, JobObserver, LocalStore, Outcome, Prepared, Progress,
    ResultDir, SeriesColumns, SeriesRow, Started, Status,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, oneshot, OwnedSemaphorePermit, Semaphore};

use crate::api::{Checked, Estimate, JobOptions};
use crate::db::{now_ms, Db, JobEnd, JobRow};
use crate::error::{ApiError, IssueDto};
use crate::stream::Hub;
use crate::{AppState, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Paused => "paused",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => JobStatus::Queued,
            "running" => JobStatus::Running,
            "paused" => JobStatus::Paused,
            "completed" => JobStatus::Completed,
            "failed" => JobStatus::Failed,
            "cancelled" => JobStatus::Cancelled,
            _ => return None,
        })
    }

    /// Queued, running or paused: the job still holds (or waits for) a device.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            JobStatus::Queued | JobStatus::Running | JobStatus::Paused
        )
    }
}

/// Why a job failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FailReason {
    /// The solution stopped being finite (the frames before stay valid).
    Diverged,
    /// A backend or I/O error.
    Error,
    /// The server stopped while the job was active.
    Interrupted,
}

impl FailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::Diverged => "diverged",
            FailReason::Error => "error",
            FailReason::Interrupted => "interrupted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    pub step: u64,
    pub total: u64,
    /// Simulated time (s).
    pub sim_time: f64,
    pub percent: f64,
    pub mlups: f64,
    /// `(mass − mass₀) / mass₀` at the last sample.
    pub mass_drift: f64,
    /// Lattice Mach number at the last sample.
    pub max_mach: f64,
}

impl From<&Progress> for ProgressDto {
    fn from(p: &Progress) -> Self {
        Self {
            step: p.step,
            total: p.total,
            sim_time: p.sim_time,
            percent: if p.total > 0 {
                100.0 * p.step as f64 / p.total as f64
            } else {
                100.0
            },
            mlups: p.mlups,
            mass_drift: p.mass_drift,
            max_mach: p.max_mach,
        }
    }
}

/// Messages to viewers (WebSocket text frames), tagged by `type`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    Status {
        status: JobStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<FailReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Progress(ProgressDto),
    /// The series rows sampled since the last `series` message (sent before each `progress`).
    Series {
        rows: Arc<SeriesRows>,
        /// Some rows were missed (the viewer fell behind): read the files again.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        resync: bool,
    },
    Log {
        message: String,
    },
    /// The job is over; nothing follows.
    #[serde(rename_all = "camelCase")]
    Done {
        status: JobStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<FailReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        steps_done: u64,
        frames: u32,
    },
}

/// A job as the API reports it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub status: JobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<FailReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub scene: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub options: serde_json::Value,
    pub steps_total: u64,
    pub steps_done: u64,
    /// Output frames so far.
    pub frames: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressDto>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<IssueDto>,
    /// Why better engines were skipped (`auto`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mlups: Option<f64>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
}

impl JobView {
    /// A job known only from the database (e.g. from an earlier server run).
    pub fn from_row(r: JobRow) -> Self {
        Self {
            status: JobStatus::parse(&r.status).unwrap_or(JobStatus::Failed),
            reason: match r.reason.as_deref() {
                Some("diverged") => Some(FailReason::Diverged),
                Some("interrupted") => Some(FailReason::Interrupted),
                Some(_) => Some(FailReason::Error),
                None => None,
            },
            message: r.message,
            scene: r.scene_name,
            scene_id: r.scene_id,
            backend: r.backend,
            device: r.device,
            options: serde_json::from_str(&r.options_json).unwrap_or_default(),
            steps_total: r.steps_total as u64,
            steps_done: r.steps_done as u64,
            frames: r.frames as u32,
            progress: None,
            warnings: Vec::new(),
            notes: Vec::new(),
            estimate: None,
            mlups: r.mlups,
            created_at: r.created_at,
            started_at: r.started_at,
            finished_at: r.finished_at,
            id: r.id,
        }
    }

    pub(crate) fn done_event(&self) -> Event {
        Event::Done {
            status: self.status,
            reason: self.reason,
            message: self.message.clone(),
            steps_done: self.steps_done,
            frames: self.frames,
        }
    }
}

/// Rows of the three series, `[step, time, …]` each (the columns of `hello.series`). `NaN`
/// becomes `null` in JSON.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SeriesRows {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub forces: Vec<Vec<f64>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<Vec<f64>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Vec<f64>>,
    /// One row per stored frame (`heat.csv`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub heat: Vec<Vec<f64>>,
}

impl SeriesRows {
    fn push(&mut self, row: &SeriesRow) {
        self.forces.push(row.forces.clone());
        self.probes.push(row.probes.clone());
        self.diagnostics.push(row.diagnostics.clone());
    }

    fn is_empty(&self) -> bool {
        self.forces.is_empty()
            && self.probes.is_empty()
            && self.diagnostics.is_empty()
            && self.heat.is_empty()
    }
}

/// Grid and units of a running job, for the stream's `hello`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GridInfo {
    pub width: u32,
    pub height: u32,
    /// m
    pub dx: f64,
    /// s
    pub dt: f64,
    pub thermal: bool,
}

/// One job, shared by the handlers, its queue task, its threads and its viewers.
pub struct Job {
    pub id: String,
    view: Mutex<JobView>,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<Event>,
    pub(crate) grid: OnceLock<GridInfo>,
    /// The series' columns, once the job has started.
    pub(crate) series: OnceLock<SeriesColumns>,
    /// Rows sampled since the last `series` event.
    pending_rows: Mutex<SeriesRows>,
    pub(crate) hub: Hub,
    /// Delete the results once the job has stopped.
    purge: AtomicBool,
    last_progress: Mutex<Option<Instant>>,
}

/// Events kept for a viewer that falls behind; past that it gets `Lagged` and resynchronises
/// from the job's view.
const EVENT_CAPACITY: usize = 64;
/// At most this many progress events per second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

impl Job {
    pub fn view(&self) -> JobView {
        self.view.lock().unwrap().clone()
    }

    pub fn status(&self) -> JobStatus {
        self.view.lock().unwrap().status
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// Changes the view under its lock (held only for `f`, never across an `.await`).
    fn update<T>(&self, f: impl FnOnce(&mut JobView) -> T) -> T {
        f(&mut self.view.lock().unwrap())
    }

    pub(crate) fn emit(&self, e: Event) {
        // `send` fails only when nobody is subscribed: nothing to do then.
        let _ = self.events.send(e);
    }

    /// Sends the rows sampled since the last `series` event, if any (before a `progress` and
    /// before `done`). They are on disk already: the job flushes the files first.
    pub(crate) fn emit_series(&self) {
        let rows = std::mem::take(&mut *self.pending_rows.lock().unwrap());
        if !rows.is_empty() {
            self.emit(Event::Series {
                rows: Arc::new(rows),
                resync: false,
            });
        }
    }

    pub(crate) fn send(&self, cmd: Command) {
        // Fails only when the job's loop is gone (the job is over): nothing to do then.
        let _ = self.commands.send(cmd);
    }
}

/// Reports a job's progress from its threads: updates the view and tells the viewers.
struct Reporter {
    job: Arc<Job>,
    db: Db,
    /// The tokio runtime, to run async database calls from the job's (non-async) thread.
    rt: tokio::runtime::Handle,
}

impl JobObserver for Reporter {
    fn started(&self, s: &Started) {
        let d = s.domain;
        let _ = self.job.grid.set(GridInfo {
            width: d.width,
            height: d.height,
            dx: d.units.dx,
            dt: d.units.dt,
            thermal: d.physics.thermal(),
        });
        let _ = self.job.series.set(s.series.clone());
    }

    fn sample(&self, row: &SeriesRow) {
        self.job.pending_rows.lock().unwrap().push(row);
    }

    fn progress(&self, p: &Progress) {
        let dto = ProgressDto::from(p);
        self.job.update(|v| {
            v.steps_done = p.step;
            v.progress = Some(dto);
        });
        let mut last = self.job.last_progress.lock().unwrap();
        if last.is_none_or(|t| t.elapsed() >= PROGRESS_INTERVAL) {
            *last = Some(Instant::now());
            self.job.emit_series();
            self.job.emit(Event::Progress(dto));
        }
    }

    fn paused(&self, paused: bool) {
        let status = if paused {
            JobStatus::Paused
        } else {
            JobStatus::Running
        };
        self.job.update(|v| v.status = status);
        self.job.emit(Event::Status {
            status,
            reason: None,
            message: None,
        });
        let (db, id) = (self.db.clone(), self.job.id.clone());
        self.rt.spawn(async move {
            if let Err(e) = db.set_status(&id, status.as_str()).await {
                tracing::warn!("job {id}: cannot record the status: {e:#}");
            }
        });
    }

    fn wants_frames(&self) -> bool {
        true
    }

    fn stream_specs(
        &self,
        frame_fields: &[OutputField],
        width: u32,
        height: u32,
    ) -> Vec<SampleSpec> {
        self.job.hub.stream_specs(frame_fields, width, height)
    }

    fn frame(&self, f: &FrameEvent) {
        if let Some(i) = f.index {
            self.job.update(|v| v.frames = v.frames.max(i + 1));
        }
        if let Some(row) = f.heat {
            // Already on disk (the output thread flushed it first): sent with the next progress.
            self.job
                .pending_rows
                .lock()
                .unwrap()
                .heat
                .push(row.to_vec());
        }
        self.job.hub.publish(&self.job.id, f);
    }

    fn log(&self, message: &str) {
        self.job.emit(Event::Log {
            message: message.to_string(),
        });
    }
}

/// Everything a queued job needs to start.
struct Work {
    prepared: Prepared,
    choice: SolverChoice,
    dir: ResultDir,
    commands: mpsc::Receiver<Command>,
    slot: Arc<Semaphore>,
    max_stored_bytes: u64,
}

pub struct JobManager {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    /// One semaphore per physical device: CUDA and wgpu on the same GPU share one permit.
    slots: HashMap<String, Arc<Semaphore>>,
    pub(crate) db: Db,
    pub(crate) store: LocalStore,
    shutting_down: AtomicBool,
}

/// The job slots: one per GPU, `cpu_jobs` for the CPU.
fn slots(inventory: &Inventory, cpu_jobs: usize) -> HashMap<String, Arc<Semaphore>> {
    let mut slots = HashMap::new();
    for e in inventory.engines().filter(|e| e.available) {
        if let Some(pd) = &e.physical_device {
            let permits = if e.kind == EngineKind::Cpu {
                cpu_jobs
            } else {
                1
            };
            slots
                .entry(pd.clone())
                .or_insert_with(|| Arc::new(Semaphore::new(permits)));
        }
    }
    slots
}

impl JobManager {
    pub async fn new(config: &Config, inventory: &Inventory, db: Db) -> anyhow::Result<Self> {
        let interrupted = db.mark_interrupted().await?;
        if interrupted > 0 {
            tracing::warn!(
                "{interrupted} job(s) were active when the server stopped: marked interrupted"
            );
        }
        Ok(Self {
            jobs: Mutex::default(),
            slots: slots(inventory, config.cpu_jobs.max(1)),
            db,
            store: LocalStore::new(config.results_dir())?,
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    /// The live view of a job of this server run, or its database row.
    pub async fn view(&self, id: &str) -> Result<JobView, ApiError> {
        if let Some(job) = self.get(id) {
            return Ok(job.view());
        }
        match self.db.job(id).await? {
            Some(row) => Ok(JobView::from_row(row)),
            None => Err(ApiError::NotFound(format!("no job {id}"))),
        }
    }

    fn queued(&self) -> usize {
        self.jobs
            .lock()
            .unwrap()
            .values()
            .filter(|j| j.status() == JobStatus::Queued)
            .count()
    }

    /// The latest completed job of the same scene and options whose results are still on disk.
    pub async fn find_reusable(&self, scene_hash: &str) -> Result<Option<JobView>, ApiError> {
        let Some(row) = self.db.latest_completed(scene_hash).await? else {
            return Ok(None);
        };
        if !std::path::Path::new(&row.result_dir).is_dir() {
            return Ok(None);
        }
        // A job of this run is still in memory, with its live view; otherwise the row is enough.
        Ok(Some(match self.get(&row.id) {
            Some(live) => live.view(),
            None => JobView::from_row(row),
        }))
    }

    /// Registers a checked scene as a job and queues it on its engine's device.
    pub async fn submit(
        &self,
        state: &AppState,
        checked: Checked,
        options: &JobOptions,
        scene_id: Option<String>,
        scene_hash: String,
    ) -> Result<JobView, ApiError> {
        let config = &state.inner.config;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(ApiError::Conflict("the server is shutting down".into()));
        }
        if self.queued() >= config.limits.max_queued_jobs {
            return Err(ApiError::TooManyRequests(format!(
                "{} jobs are already waiting for a device; try again later",
                config.limits.max_queued_jobs
            )));
        }
        let inventory = &state.inner.inventory;
        let engine = inventory
            .engine(checked.engine)
            .ok_or_else(|| anyhow!("engine {} vanished", checked.engine))?;
        let slot = inventory
            .info(checked.engine)
            .and_then(|i| i.physical_device.as_ref())
            .and_then(|pd| self.slots.get(pd))
            .cloned()
            .ok_or_else(|| anyhow!("no job slot for engine {}", checked.engine))?;
        let choice = SolverChoice::new(engine.clone(), &config.engine);

        // A UUID v7 starts with the time: ids (and result directories) sort by creation.
        let id = uuid::Uuid::now_v7().to_string();
        let dir = self.store.create(&id)?;
        let scene_json = &checked.prepared.scene_json;
        let cells = checked.estimate.cells;
        let row = JobRow {
            id: id.clone(),
            scene_id: scene_id.clone(),
            scene_name: checked.prepared.scene.name.clone(),
            scene_sha256: format!("{:x}", Sha256::digest(scene_json.as_bytes())),
            scene_hash: Some(scene_hash),
            status: JobStatus::Queued.as_str().into(),
            reason: None,
            message: None,
            backend: checked.engine.name().into(),
            device: choice.device.clone(),
            options_json: serde_json::to_string(options).map_err(anyhow::Error::from)?,
            steps_done: 0,
            steps_total: checked.prepared.total_steps() as i64,
            frames: 0,
            mlups: None,
            created_at: now_ms(),
            started_at: None,
            finished_at: None,
            result_dir: dir.path().display().to_string(),
        };
        if let Err(e) = self.db.insert_job(&row).await {
            let _ = self.store.delete(&id);
            return Err(e.into());
        }
        let view = JobView {
            options: serde_json::to_value(options).unwrap_or_default(),
            warnings: checked.warnings,
            notes: checked.notes,
            estimate: Some(checked.estimate),
            ..JobView::from_row(row)
        };
        let (tx, rx) = mpsc::channel();
        let job = Arc::new(Job {
            id: id.clone(),
            view: Mutex::new(view.clone()),
            commands: tx,
            events: broadcast::channel(EVENT_CAPACITY).0,
            grid: OnceLock::new(),
            series: OnceLock::new(),
            pending_rows: Mutex::new(SeriesRows::default()),
            hub: Hub::default(),
            purge: AtomicBool::new(false),
            last_progress: Mutex::new(None),
        });
        self.jobs.lock().unwrap().insert(id.clone(), job.clone());
        tracing::info!(
            "job {id}: queued on {} ({} cells, {} steps)",
            choice.describe(),
            cells,
            view.steps_total
        );
        let work = Work {
            prepared: checked.prepared,
            choice,
            dir,
            commands: rx,
            slot,
            max_stored_bytes: config.limits.max_stored_bytes,
        };
        // A task per job: waiting for the device costs nothing while it waits (it is a state
        // machine, not a thread; in C#, an `async` method awaiting `SemaphoreSlim.WaitAsync`).
        tokio::spawn(run_job(state.clone(), job, work));
        Ok(view)
    }

    /// Pause (`true`) or resume a running job; applied between batches.
    pub fn pause(&self, id: &str, pause: bool) -> Result<JobView, ApiError> {
        let job = self
            .get(id)
            .ok_or_else(|| ApiError::NotFound(format!("no active job {id}")))?;
        match (job.status(), pause) {
            (JobStatus::Running, true) => job.send(Command::Pause),
            (JobStatus::Paused, false) => job.send(Command::Resume),
            // Already there.
            (JobStatus::Paused, true) | (JobStatus::Running, false) => {}
            (s, _) => {
                return Err(ApiError::Conflict(format!(
                    "the job is {}: only a running job can be paused or resumed",
                    s.as_str()
                )))
            }
        }
        Ok(job.view())
    }

    /// Cancels a job: at once if it is still queued, otherwise between two batches.
    pub async fn cancel(&self, id: &str) -> Result<JobView, ApiError> {
        let job = self
            .get(id)
            .ok_or_else(|| ApiError::NotFound(format!("no active job {id}")))?;
        let was_queued = job.update(|v| {
            if v.status == JobStatus::Queued {
                v.status = JobStatus::Cancelled;
                v.message = Some("cancelled before it started".into());
                v.finished_at = Some(now_ms());
                true
            } else {
                false
            }
        });
        if was_queued {
            // The queue task sees the status when it gets the device, and gives it back.
            let view = job.view();
            self.db
                .finish_job(
                    id,
                    &JobEnd {
                        status: view.status.as_str(),
                        reason: None,
                        message: view.message.as_deref(),
                        steps_done: 0,
                        frames: 0,
                        mlups: None,
                    },
                )
                .await?;
            job.emit(view.done_event());
            self.after_end(&job).await;
        } else if job.status().is_active() {
            job.send(Command::Cancel);
        }
        Ok(job.view())
    }

    /// Deletes a job's results and its row. An active job is cancelled first and purged once it
    /// has stopped.
    pub async fn purge(&self, id: &str) -> Result<bool, ApiError> {
        if let Some(job) = self.get(id) {
            if job.status().is_active() {
                job.purge.store(true, Ordering::SeqCst);
                self.cancel(id).await?;
                if job.status().is_active() {
                    return Ok(false);
                }
            }
        } else if self.db.job(id).await?.is_none() {
            return Err(ApiError::NotFound(format!("no job {id}")));
        }
        self.delete(id).await?;
        Ok(true)
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.store.delete(id)?;
        self.db.delete_job(id).await?;
        self.jobs.lock().unwrap().remove(id);
        Ok(())
    }

    /// Clean-up once a job is over.
    async fn after_end(&self, job: &Arc<Job>) {
        job.hub.close();
        if job.purge.load(Ordering::SeqCst) {
            if let Err(e) = self.delete(&job.id).await {
                tracing::warn!("job {}: cannot purge: {e:#}", job.id);
            }
        }
    }

    /// Stops every job: queued ones are marked interrupted, running ones are cancelled and
    /// recorded as interrupted. Waits at most `timeout` for them to stop.
    pub async fn shutdown(&self, timeout: Duration) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let jobs: Vec<Arc<Job>> = self.jobs.lock().unwrap().values().cloned().collect();
        for job in &jobs {
            let was_queued = job.update(|v| {
                let q = v.status == JobStatus::Queued;
                if q {
                    v.status = JobStatus::Failed;
                    v.reason = Some(FailReason::Interrupted);
                    v.message = Some("the server stopped before the job started".into());
                }
                q
            });
            if was_queued {
                let view = job.view();
                let end = JobEnd {
                    status: view.status.as_str(),
                    reason: view.reason.map(FailReason::as_str),
                    message: view.message.as_deref(),
                    steps_done: 0,
                    frames: 0,
                    mlups: None,
                };
                if let Err(e) = self.db.finish_job(&job.id, &end).await {
                    tracing::warn!("job {}: cannot record the end: {e:#}", job.id);
                }
                job.emit(view.done_event());
                job.hub.close();
            } else if job.status().is_active() {
                job.send(Command::Cancel);
            }
        }
        let deadline = Instant::now() + timeout;
        while jobs.iter().any(|j| j.status().is_active()) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// The life of one job after submission: wait for the device, run on a dedicated thread, record
/// the outcome.
async fn run_job(state: AppState, job: Arc<Job>, work: Work) {
    let Work {
        mut prepared,
        choice,
        dir,
        commands,
        slot,
        max_stored_bytes,
    } = work;
    // FIFO: permits go to waiters in the order they asked.
    let permit: OwnedSemaphorePermit = match slot.acquire_owned().await {
        Ok(p) => p,
        Err(_) => return, // the semaphore is never closed
    };
    // Start unless it was cancelled (or the server began to stop) while queued; checked under the
    // view's lock, so a concurrent cancel either sees `running` or wins.
    let started_at = now_ms();
    let started = job.update(|v| {
        let q = v.status == JobStatus::Queued;
        if q {
            v.status = JobStatus::Running;
            v.started_at = Some(started_at);
        }
        q
    });
    if !started {
        return; // dropping the permit hands the device to the next job
    }
    let jobs = &state.inner.jobs;
    if let Err(e) = jobs.db.start_job(&job.id, started_at).await {
        tracing::warn!("job {}: cannot record the start: {e:#}", job.id);
    }
    job.emit(Event::Status {
        status: JobStatus::Running,
        reason: None,
        message: None,
    });
    tracing::info!("job {}: running on {}", job.id, choice.describe());

    prepared.options.max_stored_bytes = Some(max_stored_bytes);
    let reporter = Arc::new(Reporter {
        job: job.clone(),
        db: jobs.db.clone(),
        rt: tokio::runtime::Handle::current(),
    });
    let ctx = JobContext::new(
        job.id.clone(),
        format!("cfd-server {}", env!("CARGO_PKG_VERSION")),
    );
    // The job's result comes back through a oneshot channel (≈ a `TaskCompletionSource`); if the
    // thread panics, the sender is dropped and `await` returns an error instead.
    let (done_tx, done_rx) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(format!("job-{}", &job.id[job.id.len().saturating_sub(8)..]))
        .spawn(move || {
            let _permit = permit; // released when this thread ends, whatever happens
            let r = cfd_job::run(prepared, &choice, &dir, &ctx, reporter, &commands);
            let _ = done_tx.send(r);
        });
    let result = match spawned {
        Ok(_) => done_rx
            .await
            .unwrap_or_else(|_| Err(anyhow!("the job's thread panicked"))),
        Err(e) => Err(anyhow!("cannot start the job's thread: {e}")),
    };
    finish(&state, &job, result).await;
}

/// Records how a job ended: view, database, throughput estimate, viewers.
async fn finish(state: &AppState, job: &Arc<Job>, result: anyhow::Result<Outcome>) {
    let jobs = &state.inner.jobs;
    let interrupted = jobs.shutting_down.load(Ordering::SeqCst);
    let (status, reason, message) = match &result {
        Ok(o) => match o.status {
            Status::Completed => (JobStatus::Completed, None, None),
            Status::Diverged => (
                JobStatus::Failed,
                Some(FailReason::Diverged),
                o.message.clone(),
            ),
            Status::Cancelled if interrupted => (
                JobStatus::Failed,
                Some(FailReason::Interrupted),
                Some("the server stopped while the job was running".into()),
            ),
            Status::Cancelled => (JobStatus::Cancelled, None, o.message.clone()),
        },
        Err(e) => (
            JobStatus::Failed,
            Some(FailReason::Error),
            Some(format!("{e:#}")),
        ),
    };
    let mlups = result
        .as_ref()
        .ok()
        .map(|o| o.timings.mlups)
        .filter(|m| *m > 0.0);
    let view = job.update(|v| {
        v.status = status;
        v.reason = reason;
        v.message = message.clone();
        v.finished_at = Some(now_ms());
        if let Ok(o) = &result {
            v.steps_done = o.steps_done;
            v.frames = o.frames;
        }
        v.mlups = mlups;
        v.clone()
    });
    match &result {
        Ok(_) => tracing::info!("job {}: {}", job.id, status.as_str()),
        Err(e) => tracing::warn!("job {}: failed: {e:#}", job.id),
    }
    let end = JobEnd {
        status: status.as_str(),
        reason: reason.map(FailReason::as_str),
        message: message.as_deref(),
        steps_done: view.steps_done,
        frames: view.frames,
        mlups: if status == JobStatus::Completed {
            mlups
        } else {
            None
        },
    };
    if let Err(e) = jobs.db.finish_job(&job.id, &end).await {
        tracing::warn!("job {}: cannot record the end: {e:#}", job.id);
    }
    if let (JobStatus::Completed, Some(m), Some(kind)) = (
        status,
        mlups,
        EngineKind::BY_PREFERENCE
            .into_iter()
            .find(|k| k.name() == view.backend),
    ) {
        state.inner.record_mlups(kind, m);
    }
    // The last rows (sampled after the last progress event), then the end.
    job.emit_series();
    job.emit(view.done_event());
    jobs.after_end(job).await;
}
