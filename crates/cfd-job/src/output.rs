//! The output thread of a run: converts copied lattice fields to physical fields, encodes and
//! writes the stored frames, and hands every frame to the observer (the server encodes its stream
//! frames there).
//!
//! It is fed by a **bounded** channel (`sync_channel(2)`): the solver keeps stepping while a frame
//! is converted and written, and if the disk cannot keep up the solver waits, so stored frames are
//! never dropped. (In C# terms: a consumer `Thread` reading a `BlockingCollection` with a bounded
//! capacity.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{anyhow, Context, Result};
use cfd_core::domain::Domain;
use cfd_core::solver::{FieldSet, LatticeFields, SampleRequest};
use cfd_io::frame::{Dtype, Field, FieldDesc, Frame};

use crate::output_field_name;
use crate::runner::{FrameEvent, JobObserver};
use crate::store::ResultDir;

/// Frames queued between the solver and the output thread.
const QUEUE: usize = 2;

/// One frame for the output thread.
pub(crate) struct OutputJob {
    /// Output index; `None` for a snapshot taken only for viewers.
    pub index: Option<u32>,
    /// Write it to `frames/`.
    pub store: bool,
    pub step: u64,
    pub fields: LatticeFields,
}

/// What converting lattice fields to physical ones needs from the domain: everything but the
/// boundary tables (a few bytes per cell instead of a full copy).
fn sampling_domain(d: &Domain) -> Domain {
    Domain {
        flags: d.flags.clone(),
        bc_slot: Vec::new(),
        bc_params: Vec::new(),
        slot_element: Vec::new(),
        probes: Vec::new(),
        issues: Vec::new(),
        // The remaining fields are `Copy`: they are copied out of `*d`.
        ..*d
    }
}

/// What the output thread stored.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Stored {
    pub frames: u32,
    pub bytes: u64,
    /// Storing stopped at the size limit.
    pub limit_reached: bool,
}

pub(crate) struct OutputThread {
    tx: Option<SyncSender<OutputJob>>,
    handle: Option<JoinHandle<Result<Stored>>>,
    /// Set on cancel: frames still queued are dropped instead of written.
    abandon: Arc<AtomicBool>,
}

/// Why a frame could not be queued without waiting.
pub(crate) enum TrySend {
    Queued,
    /// The queue is full; the frame is handed back.
    Full(OutputJob),
}

pub(crate) struct OutputConfig {
    pub dir: ResultDir,
    pub job_id: String,
    pub req: SampleRequest,
    pub dtype: Dtype,
    /// Stop storing frames (not the run) past this many bytes.
    pub max_stored_bytes: Option<u64>,
}

impl OutputThread {
    pub fn spawn(
        domain: &Domain,
        config: OutputConfig,
        observer: Arc<dyn JobObserver>,
    ) -> Result<Self> {
        let domain = sampling_domain(domain);
        let (tx, rx) = sync_channel::<OutputJob>(QUEUE);
        let abandon = Arc::new(AtomicBool::new(false));
        let abandoned = abandon.clone();
        // `move` hands ownership of everything the closure uses to the new thread (the domain
        // copy, the receiver, the observer's `Arc`): nothing is shared but the observer.
        let handle = std::thread::Builder::new()
            .name(format!("output-{}", config.job_id))
            .spawn(move || -> Result<Stored> {
                let mut stored = Stored::default();
                // The loop ends when every sender is dropped (the run is over).
                for job in rx {
                    if abandoned.load(Ordering::Relaxed) {
                        continue; // cancelled: drain the queue without writing
                    }
                    let fs = FieldSet::from_lattice(&domain, &job.fields, job.step, &config.req);
                    let mut written = false;
                    if let (true, Some(index), false) = (job.store, job.index, stored.limit_reached)
                    {
                        let bytes = encode_frame(&config, index, &fs)?;
                        let size = bytes.len() as u64;
                        if config
                            .max_stored_bytes
                            .is_some_and(|max| stored.bytes + size > max)
                        {
                            stored.limit_reached = true;
                            observer.log(&format!(
                                "stored frames reached the size limit ({} MiB) at frame {index}; \
                                 the run goes on without storing frames",
                                stored.bytes >> 20
                            ));
                        } else {
                            let path = config.dir.frame_path(index);
                            std::fs::write(&path, bytes)
                                .with_context(|| format!("cannot write {}", path.display()))?;
                            stored.frames += 1;
                            stored.bytes += size;
                            written = true;
                        }
                    }
                    observer.frame(&FrameEvent {
                        index: job.index,
                        stored: written,
                        fields: &fs,
                    });
                }
                Ok(stored)
            })
            .context("cannot start the output thread")?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            abandon,
        })
    }

    /// Queues a frame if there is room. Fails with the output thread's error if it stopped (e.g.
    /// the disk is full).
    pub fn try_send(&mut self, job: OutputJob) -> Result<TrySend> {
        let tx = self.tx.as_ref().expect("output thread running");
        match tx.try_send(job) {
            Ok(()) => Ok(TrySend::Queued),
            Err(TrySendError::Full(job)) => Ok(TrySend::Full(job)),
            // The receiver is gone: the thread returned early with an error.
            Err(TrySendError::Disconnected(_)) => Err(self
                .join()
                .err()
                .unwrap_or_else(|| anyhow!("the output thread stopped"))),
        }
    }

    /// Drops the frames still queued (the run was cancelled); the frame being written finishes.
    pub fn abandon(&self) {
        self.abandon.store(true, Ordering::Relaxed);
    }

    /// Waits for every queued frame to be written.
    pub fn finish(mut self) -> Result<Stored> {
        self.join()
    }

    fn join(&mut self) -> Result<Stored> {
        // Dropping the sender ends the thread's `for` loop once the queue is empty.
        self.tx = None;
        match self.handle.take() {
            Some(h) => h
                .join()
                .map_err(|_| anyhow!("the output thread panicked"))?,
            None => Ok(Stored::default()),
        }
    }
}

fn encode_frame(config: &OutputConfig, index: u32, fs: &FieldSet) -> Result<Vec<u8>> {
    let frame = Frame {
        job_id: config.job_id.clone(),
        frame_index: index,
        step: fs.step,
        sim_time: fs.time,
        width: fs.width,
        height: fs.height,
        fields: fs
            .fields
            .iter()
            .map(|f| Field {
                desc: FieldDesc {
                    name: output_field_name(f.field).into(),
                    components: f.components,
                    dtype: config.dtype,
                },
                values: f.values.clone(),
            })
            .collect(),
    };
    Ok(frame.encode()?)
}
