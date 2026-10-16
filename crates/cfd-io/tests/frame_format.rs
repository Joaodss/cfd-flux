use cfd_io::frame::{
    encode_raw, shuffle, unshuffle, Codec, Dtype, Field, FieldDesc, Frame, FrameError, FrameInfo,
    RawField,
};

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

#[test]
fn every_codec_round_trips() {
    for codec in [Codec::None, Codec::Zstd, Codec::ShuffleLz4] {
        let frame = sample_frame(Dtype::F32);
        let bytes = frame.encode_with(codec).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 2);
        assert_eq!(Frame::decode_header(&bytes).unwrap().codec, codec);
        assert_eq!(Frame::decode(&bytes).unwrap(), frame, "{codec:?}");
    }
}

#[test]
fn the_default_codec_is_shuffle_lz4_and_compresses_smooth_fields() {
    let frame = sample_frame(Dtype::F32);
    let bytes = frame.encode().unwrap();
    assert_eq!(
        Frame::decode_header(&bytes).unwrap().codec,
        Codec::ShuffleLz4
    );
    let raw = frame.encode_with(Codec::None).unwrap().len();
    assert!(bytes.len() < raw * 3 / 4, "{} vs {raw}", bytes.len());
}

/// A v1 frame as the previous `cfd-io` wrote it: no `codec` in the header, zstd payload.
fn encode_v1(frame: &Frame) -> Vec<u8> {
    let header = serde_json::json!({
        "jobId": frame.job_id,
        "frameIndex": frame.frame_index,
        "step": frame.step,
        "simTime": frame.sim_time,
        "width": frame.width,
        "height": frame.height,
        "fields": frame.fields.iter().map(|f| &f.desc).collect::<Vec<_>>(),
    });
    let header = serde_json::to_vec(&header).unwrap();
    let mut raw = Vec::new();
    for f in &frame.fields {
        for v in &f.values {
            match f.desc.dtype {
                Dtype::F32 => raw.extend(v.to_le_bytes()),
                Dtype::F16 => raw.extend(half::f16::from_f32(*v).to_le_bytes()),
            }
        }
    }
    let mut out = b"LFFR".to_vec();
    out.extend(1u16.to_le_bytes());
    out.extend((header.len() as u32).to_le_bytes());
    out.extend(header);
    out.extend(ruzstd::encoding::compress_to_vec(
        raw.as_slice(),
        ruzstd::encoding::CompressionLevel::Fastest,
    ));
    out
}

#[test]
fn v1_frames_are_still_read() {
    for dtype in [Dtype::F32, Dtype::F16] {
        let frame = sample_frame(dtype);
        let v1 = encode_v1(&frame);
        assert_eq!(Frame::decode_header(&v1).unwrap().codec, Codec::Zstd);
        let from_v1 = Frame::decode(&v1).unwrap();
        let from_v2 = Frame::decode(&frame.encode().unwrap()).unwrap();
        assert_eq!(from_v1, from_v2);
    }
}

#[test]
fn raw_fields_encode_like_frames() {
    let frame = sample_frame(Dtype::F16);
    let f16: Vec<u16> = frame.fields[0]
        .values
        .iter()
        .map(|v| half::f16::from_f32(*v).to_bits())
        .collect();
    let info = FrameInfo {
        job_id: frame.job_id.clone(),
        frame_index: frame.frame_index,
        step: frame.step,
        sim_time: frame.sim_time,
        width: frame.width,
        height: frame.height,
    };
    let raw = [
        RawField {
            desc: &frame.fields[0].desc,
            bytes: bytemuck::cast_slice(&f16),
        },
        RawField {
            desc: &frame.fields[1].desc,
            bytes: bytemuck::cast_slice(&frame.fields[1].values),
        },
    ];
    for codec in [Codec::None, Codec::ShuffleLz4] {
        assert_eq!(
            encode_raw(&info, &raw, codec).unwrap(),
            frame.encode_with(codec).unwrap()
        );
    }
    let short = [RawField {
        desc: &frame.fields[1].desc,
        bytes: &raw[1].bytes[..raw[1].bytes.len() - 4],
    }];
    assert!(matches!(
        encode_raw(&info, &short, Codec::ShuffleLz4),
        Err(FrameError::FieldLength { .. })
    ));
}

#[test]
fn shuffle_groups_bytes_by_position_and_inverts() {
    // Two f32-wide values: bytes 0..4 and 4..8.
    assert_eq!(
        shuffle(&[0, 1, 2, 3, 4, 5, 6, 7], 4),
        [0, 4, 1, 5, 2, 6, 3, 7]
    );
    assert_eq!(shuffle(&[0, 1, 2, 3, 4, 5], 2), [0, 2, 4, 1, 3, 5]);
    // Lengths around the 64-value blocks, and a trailing partial value.
    for width in [2, 4] {
        for len in [0, 1, 63, 64, 65, 127, 128, 1000, 4096 + 3] {
            let input: Vec<u8> = (0..len * width + width / 2)
                .map(|i| (i * 7 + i / 5) as u8)
                .collect();
            let s = shuffle(&input, width);
            let m = len;
            for i in 0..m {
                for b in 0..width {
                    assert_eq!(s[b * m + i], input[i * width + b]);
                }
            }
            assert_eq!(unshuffle(&s, width), input, "width {width}, {len} values");
        }
    }
}

#[test]
fn corrupt_lz4_payloads_are_errors() {
    let bytes = sample_frame(Dtype::F32).encode().unwrap();
    let header_end = 10 + u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    // A block length past the end.
    let mut long = bytes.clone();
    long[header_end..header_end + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(Frame::decode(&long), Err(FrameError::Truncated)));
    // Garbage inside a block.
    let mut garbage = bytes.clone();
    for b in &mut garbage[header_end + 4..header_end + 40] {
        *b = 0xff;
    }
    assert!(Frame::decode(&garbage).is_err());
    // Bytes after the last block.
    let mut extra = bytes;
    extra.push(0);
    assert!(Frame::decode(&extra).is_err());
}
