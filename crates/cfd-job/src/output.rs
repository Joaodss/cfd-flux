//! The output thread of a run: converts copied lattice fields to physical fields, encodes and
//! writes the stored frames, and hands every frame to the observer (the server encodes its stream
//! frames there).
//!
//! It is fed by a **bounded** channel (`sync_channel(2)`): the solver keeps stepping while a frame
//! is converted and written, and if the disk cannot keep up the solver waits, so stored frames are
//! never dropped. (In C# terms: a consumer `Thread` reading a `BlockingCollection` with a bounded
//! capacity.)

use std::sync::mpsc::{sync_channel, SyncSender};
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

pub(crate) struct OutputThread {
    tx: Option<SyncSender<OutputJob>>,
    handle: Option<JoinHandle<Result<()>>>,
}

pub(crate) struct OutputConfig {
    pub dir: ResultDir,
    pub job_id: String,
    pub req: SampleRequest,
    pub dtype: Dtype,
}

impl OutputThread {
    pub fn spawn(
        domain: &Domain,
        config: OutputConfig,
        observer: Arc<dyn JobObserver>,
    ) -> Result<Self> {
        let domain = sampling_domain(domain);
        let (tx, rx) = sync_channel::<OutputJob>(QUEUE);
        // `move` hands ownership of everything the closure uses to the new thread (the domain
        // copy, the receiver, the observer's `Arc`): nothing is shared but the observer.
        let handle = std::thread::Builder::new()
            .name(format!("output-{}", config.job_id))
            .spawn(move || -> Result<()> {
                // The loop ends when every sender is dropped (the run is over).
                for job in rx {
                    let fs = FieldSet::from_lattice(&domain, &job.fields, job.step, &config.req);
                    if let (true, Some(index)) = (job.store, job.index) {
                        write_frame(&config, index, &fs)?;
                    }
                    observer.frame(&FrameEvent {
                        index: job.index,
                        stored: job.store,
                        fields: &fs,
                    });
                }
                Ok(())
            })
            .context("cannot start the output thread")?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
        })
    }

    /// Queues a frame, waiting while the queue is full. Fails with the output thread's error if it
    /// stopped (e.g. the disk is full).
    pub fn send(&mut self, job: OutputJob) -> Result<()> {
        let tx = self.tx.as_ref().expect("output thread running");
        if tx.send(job).is_err() {
            // The receiver is gone: the thread returned early with an error.
            return Err(self
                .join()
                .err()
                .unwrap_or_else(|| anyhow!("the output thread stopped")));
        }
        Ok(())
    }

    /// Waits for every queued frame to be written.
    pub fn finish(mut self) -> Result<()> {
        self.join()
    }

    fn join(&mut self) -> Result<()> {
        // Dropping the sender ends the thread's `for` loop once the queue is empty.
        self.tx = None;
        match self.handle.take() {
            Some(h) => h
                .join()
                .map_err(|_| anyhow!("the output thread panicked"))?,
            None => Ok(()),
        }
    }
}

fn write_frame(config: &OutputConfig, index: u32, fs: &FieldSet) -> Result<()> {
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
    let path = config.dir.frame_path(index);
    std::fs::write(&path, frame.encode()?)
        .with_context(|| format!("cannot write {}", path.display()))
}
