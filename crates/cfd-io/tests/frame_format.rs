use cfd_io::frame::{Dtype, Field, FieldDesc, Frame, FrameError};

fn sample_frame(dtype: Dtype) -> Frame {
    let (w, h) = (64u32, 32u32);
    let n = (w * h) as usize;
    let velocity: Vec<f32> = (0..n * 2).map(|i| ((i as f32) * 0.01).sin()).collect();
    let pressure: Vec<f32> = (0..n)
        .map(|i| 101.0 + (i % w as usize) as f32 * 0.5)
        .collect();
    Frame {
        job_id: "job-1".into(),
        frame_index: 7,
        step: 7000,
        sim_time: 0.07,
        width: w,
        height: h,
        fields: vec![
            Field {
                desc: FieldDesc {
                    name: "velocity".into(),
                    components: 2,
                    dtype,
                },
                values: velocity,
            },
            Field {
                desc: FieldDesc {
                    name: "pressure".into(),
                    components: 1,
                    dtype: Dtype::F32,
                },
                values: pressure,
            },
        ],
    }
}

#[test]
fn f32_round_trip_is_exact() {
    let frame = sample_frame(Dtype::F32);
    let decoded = Frame::decode(&frame.encode().unwrap()).unwrap();
    assert_eq!(decoded, frame);
}

#[test]
fn f16_round_trip_within_half_precision() {
    let frame = sample_frame(Dtype::F16);
    let decoded = Frame::decode(&frame.encode().unwrap()).unwrap();
    for (a, b) in frame.fields[0].values.iter().zip(&decoded.fields[0].values) {
        assert!((a - b).abs() <= 1e-3, "{a} vs {b}");
    }
    assert_eq!(decoded.fields[1], frame.fields[1]);
}

#[test]
fn f16_frames_are_smaller() {
    let f32_len = sample_frame(Dtype::F32).encode().unwrap().len();
    let f16_len = sample_frame(Dtype::F16).encode().unwrap().len();
    assert!(f16_len < f32_len, "{f16_len} >= {f32_len}");
}

#[test]
fn rejects_corrupt_input() {
    let bytes = sample_frame(Dtype::F32).encode().unwrap();

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(matches!(Frame::decode(&bad_magic), Err(FrameError::Magic)));

    let mut bad_version = bytes.clone();
    bad_version[4] = 99;
    assert!(matches!(
        Frame::decode(&bad_version),
        Err(FrameError::Version(99))
    ));

    assert!(matches!(
        Frame::decode(&bytes[..5]),
        Err(FrameError::Truncated)
    ));
    assert!(Frame::decode(&bytes[..bytes.len() - 10]).is_err());
}

#[test]
fn rejects_wrong_field_length_on_encode() {
    let mut frame = sample_frame(Dtype::F32);
    frame.fields[1].values.pop();
    assert!(matches!(
        frame.encode(),
        Err(FrameError::FieldLength { .. })
    ));
}
