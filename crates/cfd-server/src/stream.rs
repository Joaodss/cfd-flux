//! Streaming frames to viewers, with backpressure.
//!
//! Frames reach the [`Hub`] from the job's output thread. For every *format* (fields and maximum
//! size) some viewer asked for, the hub encodes one stream frame (f16, box-downsampled) and puts it
//! in a `tokio::sync::watch` channel, which holds only the **latest** value (≈ Rx's
//! `BehaviorSubject`). Each viewer's task sends the latest frame, waits for the socket, then for
//! the next change: a slow viewer skips frames, a fast one gets all of them, and the job never
//! waits for anyone. Stored frames are written by the output thread before this and are never
//! dropped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cfd_core::solver::FieldSet;
use cfd_io::frame::{Dtype, Field, FieldDesc, Frame};
use cfd_job::FrameEvent;
use serde::Serialize;
use tokio::sync::watch;

/// An encoded stream frame (LFFR, f16), shared by every viewer of its format.
pub type Encoded = Arc<Vec<u8>>;

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

    /// Encodes `f` for every format with viewers. Runs on the job's output thread; the lock is
    /// held only to read and update the bookkeeping, never while encoding, so async handlers that
    /// subscribe are not blocked by it.
    pub fn publish(&self, job_id: &str, f: &FrameEvent) {
        let ranges = field_ranges(f.fields);
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
            // Formats whose last viewer has gone are dropped here.
            s.formats.retain(|_, tx| tx.receiver_count() > 0);
            let targets: Vec<_> = s
                .formats
                .iter()
                .map(|(fmt, tx)| (fmt.clone(), tx.clone()))
                .collect();
            (targets, f.index.unwrap_or(s.last_index))
        };
        for (fmt, tx) in targets {
            match encode(job_id, index, f.fields, &fmt) {
                // `send_replace` stores the value even if every viewer left meanwhile.
                Ok(bytes) => {
                    tx.send_replace(Some(Arc::new(bytes)));
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

fn field_ranges(fs: &FieldSet) -> Vec<(String, [f32; 2])> {
    fs.fields
        .iter()
        .filter_map(|f| {
            let name = cfd_job::output_field_name(f.field).to_string();
            let c = f.components as usize;
            let mut it = f.values.chunks_exact(c).map(|v| {
                if c == 1 {
                    v[0]
                } else {
                    v.iter().map(|x| x * x).sum::<f32>().sqrt()
                }
            });
            let first = it.next()?;
            let (lo, hi) = it.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
            Some((name, [lo, hi]))
        })
        .collect()
}

/// Averages `k × k` blocks (partial blocks at the right and top edges average what they have).
/// `values` are row-major with `c` interleaved components.
pub fn downsample(values: &[f32], c: usize, w: usize, h: usize, k: usize) -> Vec<f32> {
    if k <= 1 {
        return values.to_vec();
    }
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0.0f32; ow * oh * c];
    for oy in 0..oh {
        for ox in 0..ow {
            let (x0, y0) = (ox * k, oy * k);
            let (x1, y1) = ((x0 + k).min(w), (y0 + k).min(h));
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            for comp in 0..c {
                let mut sum = 0.0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        sum += values[(y * w + x) * c + comp];
                    }
                }
                out[(oy * ow + ox) * c + comp] = sum / n;
            }
        }
    }
    out
}

/// One LFFR stream frame: the format's fields, downsampled, as f16.
pub fn encode(
    job_id: &str,
    index: u32,
    fs: &FieldSet,
    fmt: &StreamFormat,
) -> Result<Vec<u8>, cfd_io::frame::FrameError> {
    let (w, h) = (fs.width as usize, fs.height as usize);
    let k = fmt.factor(fs.width, fs.height) as usize;
    let fields = fs
        .fields
        .iter()
        .filter(|f| {
            let name = cfd_job::output_field_name(f.field);
            fmt.fields
                .as_ref()
                .is_none_or(|list| list.iter().any(|n| n == name))
        })
        .map(|f| Field {
            desc: FieldDesc {
                name: cfd_job::output_field_name(f.field).into(),
                components: f.components,
                dtype: Dtype::F16,
            },
            values: downsample(&f.values, f.components as usize, w, h, k),
        })
        .collect();
    Frame {
        job_id: job_id.to_string(),
        frame_index: index,
        step: fs.step,
        sim_time: fs.time,
        width: w.div_ceil(k) as u32,
        height: h.div_ceil(k) as u32,
        fields,
    }
    .encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfd_core::scene::OutputField;
    use cfd_core::solver::SampledField;

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

    #[test]
    fn downsampling_averages_blocks_and_partial_edges() {
        // 3 × 2 cells, 2 components (component 1 = 10 × component 0).
        let v: Vec<f32> = (0..6).flat_map(|i| [i as f32, 10.0 * i as f32]).collect();
        let d = downsample(&v, 2, 3, 2, 2);
        // Blocks: cells {0,1,3,4} and the partial column {2,5}.
        assert_eq!(d, vec![2.0, 20.0, 3.5, 35.0]);
        assert_eq!(downsample(&v, 2, 3, 2, 1), v);
    }

    #[test]
    fn stream_frames_hold_the_requested_fields_in_f16() {
        let fs = FieldSet {
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
        };
        let fmt = StreamFormat {
            fields: Some(vec!["pressure".into()]),
            max_size: 2,
        };
        let frame = Frame::decode(&encode("job", 3, &fs, &fmt).unwrap()).unwrap();
        assert_eq!((frame.width, frame.height, frame.frame_index), (2, 1, 3));
        assert_eq!(frame.fields.len(), 1);
        assert_eq!(frame.fields[0].desc.dtype, Dtype::F16);
        // Blocks {0,1,4,5} and {2,3,6,7}.
        assert_eq!(frame.fields[0].values, vec![2.5, 4.5]);
        let r: HashMap<String, [f32; 2]> = field_ranges(&fs).into_iter().collect();
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
        let fs = FieldSet {
            step: 1,
            time: 0.1,
            width: 2,
            height: 1,
            fields: vec![SampledField {
                field: OutputField::Pressure,
                components: 1,
                values: vec![1.0, 2.0],
            }],
        };
        let mut a = hub.subscribe(&fmt).unwrap();
        hub.publish(
            "job",
            &FrameEvent {
                index: Some(0),
                stored: true,
                fields: &fs,
            },
        );
        assert!(a.has_changed().unwrap());
        let late = hub.subscribe(&fmt).unwrap();
        assert!(
            late.has_changed().unwrap(),
            "the latest frame is sent at once"
        );
        assert_eq!(*a.borrow_and_update(), *late.borrow());
        hub.close();
        assert!(hub.subscribe(&fmt).is_none());
        assert!(a.has_changed().is_err(), "closed after the job");
    }
}
