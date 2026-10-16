//! How WGSL's `pack2x16float` rounds on this driver (design proposal §6): the WGSL spec lets an
//! implementation round an f32 that is not exactly representable in f16 to either neighbour, so
//! wgpu's f16 samples are only held to one f16 ulp. This test packs values chosen to expose the
//! rounding mode — exact ties, values just off a tie, subnormals, overflow — and reports whether
//! the result is round-to-nearest-even (what the CPU's `half` and CUDA's `__float2half_rn` do).
//! It asserts only the one-ulp bound; the report says whether the stricter rule holds.

use std::sync::Arc;

use cfd_core::sample::f16_bits;
use cfd_gpu::{GpuContext, GpuOptions};

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<u32>;

@compute @workgroup_size(64)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (2u * i + 1u < arrayLength(&input)) {
        output[i] = pack2x16float(vec2<f32>(input[2u * i], input[2u * i + 1u]));
    }
}
"#;

fn context() -> Option<Arc<GpuContext>> {
    let opts = GpuOptions {
        allow_software: true,
        ..Default::default()
    };
    match GpuContext::new(&opts) {
        Ok(c) => Some(c),
        Err(e) if std::env::var_os("CFD_REQUIRE_GPU").is_some() => {
            panic!("CFD_REQUIRE_GPU is set but there is no wgpu adapter: {e}")
        }
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

/// Values whose f16 rounding depends on the mode.
fn probes() -> Vec<f32> {
    let mut v = Vec::new();
    // Ties between neighbours with even and odd mantissas, around several exponents and signs.
    for e in [-14i32, -3, 0, 5, 15] {
        let ulp = 2f32.powi(e - 10);
        for m in 0..8 {
            let base = 2f32.powi(e) + m as f32 * ulp;
            for s in [1.0f32, -1.0] {
                v.push(s * (base + 0.5 * ulp)); // exact tie
                v.push(s * (base + 0.5 * ulp * (1.0 + 2f32.powi(-12)))); // just above
                v.push(s * (base + 0.5 * ulp * (1.0 - 2f32.powi(-12)))); // just below
                v.push(s * (base + 0.25 * ulp)); // a quarter
            }
        }
    }
    // Subnormal f16 ties (ulp 2⁻²⁴) and the overflow threshold.
    for m in 0..6 {
        v.push((m as f32 + 0.5) * 2f32.powi(-24));
    }
    v.extend([65504.0, 65519.0, 65520.0, 1e-9, -1e-9, 0.0, -0.0]);
    if v.len() % 2 == 1 {
        v.push(1.0);
    }
    v
}

#[test]
fn pack2x16float_rounding() {
    let Some(ctx) = context() else { return };
    use wgpu::util::DeviceExt as _;
    let input = probes();
    let device = &ctx.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("pack"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("pack"),
        layout: None,
        module: &module,
        entry_point: Some("pack"),
        compilation_options: Default::default(),
        cache: None,
    });
    let src = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = (input.len() * 2) as u64;
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: src.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out.as_entire_binding(),
            },
        ],
    });
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((input.len() as u32 / 2).div_ceil(64), 1, 1);
    }
    enc.copy_buffer_to_buffer(&out, 0, &staging, 0, bytes);
    let index = ctx.queue.submit([enc.finish()]);
    staging.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })
        .expect("poll");
    let got: Vec<u16> = bytemuck::cast_slice(&staging.get_mapped_range(..).unwrap()).to_vec();
    let want = f16_bits(&input);

    let mut differ = Vec::new();
    for (i, (&g, &w)) in got.iter().zip(&want).enumerate() {
        if g != w {
            differ.push((input[i], g, w));
        }
    }
    eprintln!(
        "pack2x16float on {}: {} of {} probes differ from round-to-nearest-even{}",
        ctx.description(),
        differ.len(),
        input.len(),
        if differ.is_empty() {
            " (it is RNE here)"
        } else {
            ""
        }
    );
    for (v, g, w) in &differ {
        eprintln!("  {v:e}: {g:#06x}, RNE {w:#06x}");
        // One ulp at most (the WGSL spec's bound), except across the overflow threshold.
        let ordered = |b: u16| (b & 0x7fff) as i32;
        assert!(
            (ordered(*g) - ordered(*w)).abs() <= 1,
            "{v:e}: more than one ulp off"
        );
    }
}
