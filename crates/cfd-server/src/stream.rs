//! Streaming frames to viewers, with backpressure.
//!
//! Frames reach the [`Hub`] from the job's output thread. For every *format* (fields and maximum
//! size) some viewer asked for, the hub encodes one stream frame (f16, box-downsampled) and puts it
//! in a `tokio::sync::watch` channel, which holds only the **latest** value (≈ Rx's
//! `BehaviorSubject`). Each viewer's task sends the latest frame, waits for the socket, then for
//! the next change: a slow viewer skips frames, a fast one gets all of them, and the job never
//! waits for anyone. Stored frames are written by the output thread before this and are never
//! dropped.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::Response;
use cfd_core::sample::{Precision, Sample, SampleData, SampleSpec};
use cfd_core::scene::OutputField;
use cfd_job::{Command, FrameEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::error::ApiError;
use crate::jobs::{Event, GridInfo, Job, JobStatus, JobView};
use crate::AppState;

/// An encoded stream frame (LFFR, f16), shared by every viewer of its format (`Bytes` is
/// reference-counted: sending it to many sockets copies nothing).
pub type Encoded = Bytes;

/// What a viewer receives: some fields, downsampled to fit a size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StreamFormat {
    /// Field names (`velocity`, `pressure`, …); `None` for every field of the frames.
    pub fields: Option<Vec<String>>,
    /// Longest side in cells.
    pub max_size: u32,
}

impl StreamFormat {
    /// The integer box-filter factor that brings `width × height` within `max_size`.
    pub fn factor(&self, width: u32, height: u32) -> u32 {
        width.max(height).div_ceil(self.max_size.max(1)).max(1)
    }

    /// The sample this format streams: the frame fields it names (in frame order; all of them
    /// without a list), box-downsampled to fit, in f16.
    pub fn spec(&self, frame_fields: &[OutputField], width: u32, height: u32) -> SampleSpec {
        let fields = frame_fields
            .iter()
            .copied()
            .filter(|f| {
                let name = cfd_job::output_field_name(*f);
                self.fields
                    .as_ref()
                    .is_none_or(|list| list.iter().any(|n| n == name))
            })
            .collect();
        SampleSpec {
            fields,
            factor: self.factor(width, height),
            precision: Precision::F16,
        }
    }
}

/// Colour-range information for viewers: min/max of each field (velocity: of its magnitude)
/// over every frame so far.
pub type Ranges = HashMap<String, [f32; 2]>;

#[derive(Default)]
struct HubState {
    formats: HashMap<StreamFormat, watch::Sender<Option<Encoded>>>,
    ranges: Ranges,
    /// Index of the last output frame (snapshots reuse it).
    last_index: u32,
    /// The run's frame fields and grid size (set by the first `stream_specs`).
    frame: Option<(Vec<OutputField>, u32, u32)>,
    closed: bool,
}

/// The stream side of one job.
#[derive(Default)]
pub struct Hub {
    state: Mutex<HubState>,
}

impl Hub {
    /// A receiver of the latest frame in `format` (the current one, if any, is marked unseen so
    /// it is sent at once). `None` once the job is over.
    pub fn subscribe(&self, format: &StreamFormat) -> Option<watch::Receiver<Option<Encoded>>> {
        let mut s = self.state.lock().unwrap();
        if s.closed {
            return None;
        }
        let tx = s
            .formats
            .entry(format.clone())
            .or_insert_with(|| watch::channel(None).0);
        let mut rx = tx.subscribe();
        if rx.borrow().is_some() {
            rx.mark_changed();
        }
        Some(rx)
    }

    pub fn ranges(&self) -> Ranges {
        self.state.lock().unwrap().ranges.clone()
    }

    /// The samples to take with the next frame: one per format with viewers (formats whose last
    /// viewer has gone are dropped here). Called by the job's thread before each frame.
    pub fn stream_specs(
        &self,
        frame_fields: &[OutputField],
        width: u32,
        height: u32,
    ) -> Vec<SampleSpec> {
        let mut s = self.state.lock().unwrap();
        s.frame = Some((frame_fields.to_vec(), width, height));
        s.formats.retain(|_, tx| tx.receiver_count() > 0);
        s.formats
            .keys()
            .map(|fmt| fmt.spec(frame_fields, width, height))
            .collect()
    }

    /// Encodes the sample of every format with viewers (computed on the device by GPU
    /// backends). Runs on the job's output thread; the lock is held only to read and update the
    /// bookkeeping, never while encoding, so async handlers that subscribe are not blocked by it.
    /// A format that appeared after the frame was requested has no sample yet: it gets the next
    /// frame (subscribing a new format also asks the job for a snapshot).
    pub fn publish(&self, job_id: &str, f: &FrameEvent) {
        let ranges = sample_ranges(f.samples);
        let (targets, index) = {
            let mut s = self.state.lock().unwrap();
            if s.closed {
                return;
            }
            for (name, [lo, hi]) in ranges {
                let r = s.ranges.entry(name).or_insert([lo, hi]);
                *r = [r[0].min(lo), r[1].max(hi)];
            }
            if let Some(i) = f.index {
                s.last_index = i;
            }
            let Some((fields, w, h)) = s.frame.clone() else {
                return;
            };
            let targets: Vec<_> = s
                .formats
                .iter()
                .map(|(fmt, tx)| (fmt.spec(&fields, w, h), tx.clone()))
                .collect();
            (targets, f.index.unwrap_or(s.last_index))
        };
        for (spec, tx) in targets {
            let Some(sample) = f.samples.iter().find(|s| s.spec == spec) else {
                continue;
            };
            match cfd_job::encode_sample(job_id, index, sample) {
                // `send_replace` stores the value even if every viewer left meanwhile.
                Ok(bytes) => {
                    tx.send_replace(Some(Bytes::from(bytes)));
                }
                Err(e) => tracing::warn!("job {job_id}: cannot encode a stream frame: {e}"),
            }
        }
    }

    /// The job is over: viewers' receivers see the channel close after the last frame.
    pub fn close(&self) {
        let mut s = self.state.lock().unwrap();
        s.closed = true;
        s.formats.clear();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatInfo {
    pub fields: Option<Vec<String>>,
    pub max_size: u32,
    /// Box-filter factor; stream frames are `ceil(width / factor) × ceil(height / factor)`.
    pub factor: u32,
    pub width: u32,
    pub height: u32,
    pub dtype: &'static str,
}

impl FormatInfo {
    pub fn new(fmt: &StreamFormat, width: u32, height: u32) -> Self {
        let k = fmt.factor(width, height);
        Self {
            fields: fmt.fields.clone(),
            max_size: fmt.max_size,
            factor: k,
            width: width.div_ceil(k),
            height: height.div_ceil(k),
            dtype: "f16",
        }
    }
}

/// Min/max of every field over the samples of a frame (velocity: of its magnitude). The stored
/// frame, when there is one, is the full-resolution sample; downsampled ones lie within it.
fn sample_ranges(samples: &[Sample]) -> Vec<(String, [f32; 2])> {
    let mut out: Vec<(String, [f32; 2])> = Vec::new();
    for f in samples.iter().flat_map(|s| &s.fields) {
        let name = cfd_job::output_field_name(f.field).to_string();
        let c = f.components as usize;
        // The f32 values are borrowed; f16 ones converted (`Cow` ≈ "borrowed or owned").
        let values: Cow<[f32]> = match &f.data {
            SampleData::F32(v) => Cow::Borrowed(v),
            other => Cow::Owned(other.to_f32()),
        };
        let mut it = values.chunks_exact(c).map(|v| {
            if c == 1 {
                v[0]
            } else {
                v.iter().map(|x| x * x).sum::<f32>().sqrt()
            }
        });
        let Some(first) = it.next() else { continue };
        let (lo, hi) = it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
        match out.iter_mut().find(|(n, _)| *n == name) {
            Some((_, r)) => *r = [r[0].min(lo), r[1].max(hi)],
            None => out.push((name, [lo, hi])),
        }
    }
    out
}

// ---- the WebSocket ------------------------------------------------------------------------

/// `?fields=velocity,vorticity&maxSize=1024`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamQuery {
    fields: Option<String>,
    max_size: Option<u32>,
}

/// `velocity, vorticity` → the names (blanks dropped).
pub fn field_list(list: &str) -> Vec<String> {
    list.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The first message of every stream.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Hello<'a> {
    r#type: &'static str,
    job: &'a JobView,
    /// `null` until the job has started.
    grid: Option<&'a GridInfo>,
    format: Option<FormatInfo>,
    /// Min/max of each field over the frames so far (velocity: of its magnitude).
    ranges: Ranges,
}

/// `GET /api/simulations/{id}/stream` (WebSocket): `hello`, then JSON events (`status`,
/// `progress` at most 10 Hz, `log`, `done`) as text messages and stream frames (LFFR, f16) as
/// binary messages. Unknown jobs are a `404` before the upgrade.
pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let view = state.inner.jobs.view(&id).await?;
    let job = state.inner.jobs.get(&id);
    let format = StreamFormat {
        fields: q.fields.as_deref().map(field_list),
        max_size: q
            .max_size
            .unwrap_or(state.inner.config.stream.stream_max_size)
            .clamp(16, 16384),
    };
    // `on_upgrade` answers `101 Switching Protocols` and runs the closure on the upgraded
    // connection, as its own task.
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(e) = viewer(socket, view, job, format).await {
            tracing::debug!("job {id}: viewer left: {e}");
        }
    }))
}

fn text<T: Serialize>(value: &T) -> Message {
    Message::Text(serde_json::to_string(value).unwrap_or_default().into())
}

/// The end of a stream: the last frame if this viewer has not had it (every frame is published
/// before `done`), then `done`.
async fn finish(
    socket: &mut WebSocket,
    rx: &watch::Receiver<Option<Encoded>>,
    last_sent: Option<&Encoded>,
    done: &Event,
) -> Result<(), axum::Error> {
    let latest = rx.borrow().clone();
    if let Some(f) = latest.filter(|f| last_sent != Some(f)) {
        socket.send(Message::Binary(f)).await?;
    }
    socket.send(text(done)).await
}

/// One viewer: forwards events and the latest frame of its format until the job is done or the
/// viewer leaves. Each `socket.send(..).await` waits for the socket: while it waits, newer frames
/// replace older ones in the `watch` channel, so a slow viewer skips frames instead of slowing
/// the job.
async fn viewer(
    mut socket: WebSocket,
    view: JobView,
    job: Option<Arc<Job>>,
    format: StreamFormat,
) -> Result<(), axum::Error> {
    let Some(job) = job else {
        // A job of an earlier server run: it is over.
        let hello = Hello {
            r#type: "hello",
            job: &view,
            grid: None,
            format: None,
            ranges: Ranges::new(),
        };
        socket.send(text(&hello)).await?;
        socket.send(text(&view.done_event())).await?;
        return socket.send(Message::Close(None)).await;
    };
    // Subscribe before taking the view, so no event falls between them.
    let mut events = job.subscribe();
    let mut frames = job.hub.subscribe(&format);
    let view = job.view();
    let grid = job.grid.get();
    let hello = Hello {
        r#type: "hello",
        job: &view,
        grid,
        format: grid.map(|g| FormatInfo::new(&format, g.width, g.height)),
        ranges: job.hub.ranges(),
    };
    socket.send(text(&hello)).await?;
    let Some(rx) = frames.as_mut().filter(|_| view.status.is_active()) else {
        socket.send(text(&view.done_event())).await?;
        return socket.send(Message::Close(None)).await;
    };
    // A new format (or a paused job) has no frame yet: ask the job for one now.
    if rx.borrow().is_none() && view.status != JobStatus::Queued {
        job.send(Command::Snapshot);
    }

    let mut last_sent: Option<Encoded> = None;
    let mut frames_open = true;
    loop {
        // `select!` waits for whichever comes first (like `Task.WhenAny`); the branches not
        // taken are dropped, which is safe for all three futures.
        tokio::select! {
            event = events.recv() => match event {
                Ok(done @ Event::Done { .. }) => {
                    finish(&mut socket, rx, last_sent.as_ref(), &done).await?;
                    break;
                }
                Ok(e) => socket.send(text(&e)).await?,
                // Fell more than the channel's capacity behind: resynchronise from the view.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let v = job.view();
                    socket
                        .send(text(&Event::Status {
                            status: v.status,
                            reason: v.reason,
                            message: v.message.clone(),
                        }))
                        .await?;
                    if let Some(p) = v.progress {
                        socket.send(text(&Event::Progress(p))).await?;
                    }
                    if !v.status.is_active() {
                        finish(&mut socket, rx, last_sent.as_ref(), &v.done_event()).await?;
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            changed = rx.changed(), if frames_open => match changed {
                Ok(()) => {
                    let latest = rx.borrow_and_update().clone();
                    if let Some(f) = latest {
                        socket.send(Message::Binary(f.clone())).await?;
                        last_sent = Some(f);
                    }
                }
                // The hub closed: `done` follows on the event channel.
                Err(_) => frames_open = false,
            },
            msg = socket.recv() => match msg {
                // Client messages are reserved for interactive mode (Phase 5b).
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Ok(()),
                Some(Ok(_)) => {}
            },
        }
    }
    socket.send(Message::Close(None)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfd_core::sample::SampleField;
    use cfd_core::solver::{FieldSet, SampledField};
    use cfd_io::frame::{Dtype, Frame};

    #[test]
    fn the_factor_fits_the_longest_side() {
        let f = |max_size| StreamFormat {
            fields: None,
            max_size,
        };
        assert_eq!(f(1024).factor(442, 84), 1);
        assert_eq!(f(1024).factor(2048, 1024), 2);
        assert_eq!(f(1024).factor(4096, 100), 4);
        assert_eq!(f(1000).factor(4096, 100), 5);
        assert_eq!(f(0).factor(3, 3), 3);
    }

    fn field_set() -> FieldSet {
        FieldSet {
            step: 10,
            time: 0.5,
            width: 4,
            height: 2,
            fields: vec![
                SampledField {
                    field: OutputField::Velocity,
                    components: 2,
                    values: vec![1.0; 16],
                },
                SampledField {
                    field: OutputField::Pressure,
                    components: 1,
                    values: (0..8).map(|i| i as f32).collect(),
                },
            ],
        }
    }

    #[test]
    fn formats_stream_their_fields_downsampled_in_f16() {
        let frame_fields = [OutputField::Velocity, OutputField::Pressure];
        let fmt = StreamFormat {
            fields: Some(vec!["pressure".into()]),
            max_size: 2,
        };
        let spec = fmt.spec(&frame_fields, 4, 2);
        assert_eq!(spec.fields, [OutputField::Pressure]);
        assert_eq!((spec.factor, spec.precision), (2, Precision::F16));
        let all = StreamFormat {
            fields: None,
            max_size: 8,
        };
        assert_eq!(all.spec(&frame_fields, 4, 2).fields, frame_fields);

        let sample = Sample::from_field_set(&field_set(), &spec);
        let frame = Frame::decode(&cfd_job::encode_sample("job", 3, &sample).unwrap()).unwrap();
        assert_eq!((frame.width, frame.height, frame.frame_index), (2, 1, 3));
        assert_eq!(frame.fields.len(), 1);
        assert_eq!(frame.fields[0].desc.dtype, Dtype::F16);
        // Blocks {0,1,4,5} and {2,3,6,7}.
        assert_eq!(frame.fields[0].values, vec![2.5, 4.5]);
        let full = Sample::from_field_set(
            &field_set(),
            &SampleSpec::full(frame_fields.to_vec(), Precision::F32),
        );
        let r: HashMap<String, [f32; 2]> = sample_ranges(&[full, sample]).into_iter().collect();
        assert_eq!(r["pressure"], [0.0, 7.0]);
        assert_eq!(r["velocity"], [2f32.sqrt(), 2f32.sqrt()]);
    }

    #[test]
    fn viewers_of_a_format_share_one_channel_and_late_ones_get_the_latest_frame() {
        let hub = Hub::default();
        let fmt = StreamFormat {
            fields: None,
            max_size: 1024,
        };
        let fields = [OutputField::Pressure];
        let mut a = hub.subscribe(&fmt).unwrap();
        // The job asks for the formats' samples before each frame.
        let specs = hub.stream_specs(&fields, 2, 1);
        assert_eq!(specs, [fmt.spec(&fields, 2, 1)]);
        let sample = Sample {
            spec: specs[0].clone(),
            step: 1,
            time: 0.1,
            width: 2,
            height: 1,
            fields: vec![SampleField {
                field: OutputField::Pressure,
                components: 1,
                data: SampleData::F16(cfd_core::sample::f16_bits(&[1.0, 2.0])),
            }],
        };
        hub.publish(
            "job",
            &FrameEvent {
                index: Some(0),
                stored: true,
                samples: &[sample],
            },
        );
        assert!(a.has_changed().unwrap());
        let late = hub.subscribe(&fmt).unwrap();
        assert!(
            late.has_changed().unwrap(),
            "the latest frame is sent at once"
        );
        assert_eq!(*a.borrow_and_update(), *late.borrow());
        assert_eq!(hub.ranges()["pressure"], [1.0, 2.0]);

        // A format without a sample in the frame (it appeared after the request) gets nothing.
        let other = StreamFormat {
            fields: None,
            max_size: 1,
        };
        let b = hub.subscribe(&other).unwrap();
        hub.publish(
            "job",
            &FrameEvent {
                index: Some(1),
                stored: true,
                samples: &[],
            },
        );
        assert!(!b.has_changed().unwrap());
        hub.close();
        assert!(hub.subscribe(&fmt).is_none());
        assert!(a.has_changed().is_err(), "closed after the job");
    }
}
