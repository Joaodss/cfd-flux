//! Shader checks that need no GPU: every module parses and validates with naga (wgpu's shader
//! compiler), and the WGSL structs have the byte layout of the Rust data uploaded into them.

use cfd_core::domain::BcParams;
use cfd_core::DomainOptions;
use cfd_lbm::LbmConfig;
use naga::valid::{Capabilities, ValidationFlags, Validator};

use crate::lbm::{flag_bytes, params_words};
use crate::prelude::{source, LBM_WGSL, REDUCE_WGSL};

fn module(kernel: &str) -> naga::Module {
    let src = source(kernel);
    let module =
        naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    module
}

/// `(size, [(member, offset)])` of a WGSL struct.
fn layout(m: &naga::Module, name: &str) -> (u32, Vec<(String, u32)>) {
    let mut layouter = naga::proc::Layouter::default();
    layouter.update(m.to_ctx()).expect("layout");
    let (handle, ty) = m
        .types
        .iter()
        .find(|(_, t)| t.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no struct {name}"));
    let naga::TypeInner::Struct { members, .. } = &ty.inner else {
        panic!("{name} is not a struct")
    };
    let offsets = members
        .iter()
        .map(|mb| (mb.name.clone().unwrap_or_default(), mb.offset))
        .collect();
    (layouter[handle].size, offsets)
}

#[test]
fn every_shader_module_validates() {
    module(LBM_WGSL);
    module(REDUCE_WGSL);
}

#[test]
fn bc_params_layout_matches_the_rust_struct() {
    let m = module(LBM_WGSL);
    let (size, members) = layout(&m, "BcParams");
    assert_eq!(size as usize, std::mem::size_of::<BcParams>());
    let offset = |name: &str| members.iter().find(|(n, _)| n == name).unwrap().1 as usize;
    assert_eq!(offset("velocity"), std::mem::offset_of!(BcParams, velocity));
    assert_eq!(offset("density"), std::mem::offset_of!(BcParams, density));
    assert_eq!(offset("theta"), std::mem::offset_of!(BcParams, theta));
    assert_eq!(
        offset("heat_flux"),
        std::mem::offset_of!(BcParams, heat_flux)
    );
}

#[test]
fn params_layout_matches_params_words() {
    let m = module(LBM_WGSL);
    let (size, members) = layout(&m, "Params");
    assert_eq!(size, 64);
    // Word index of each member in `params_words`.
    let expected = [
        ("w", 0),
        ("h", 1),
        ("n", 2),
        ("periodic", 3),
        ("omega", 4),
        ("omega_g", 6),
        ("body_force", 8),
        ("buoyancy", 10),
        ("links", 12),
    ];
    for (name, word) in expected {
        let off = members.iter().find(|(n, _)| n == name).unwrap().1;
        assert_eq!(off, 4 * word, "{name}");
    }
    let d = cfd_core::Domain::from_scene(&cfd_core::examples::channel(), &DomainOptions::default())
        .unwrap();
    let words = params_words(&d, LbmConfig::default(), [0.5, -0.25], 7);
    assert_eq!(words[2], d.width * d.height);
    assert_eq!(f32::from_bits(words[9]), -0.25);
    assert_eq!(words[12], 7);
}

#[test]
fn flags_are_packed_little_endian() {
    let bytes = flag_bytes(&[0x1234, 0xabcd, 0x0001]);
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let unpack = |c: usize| (words[c >> 1] >> ((c & 1) * 16)) & 0xffff;
    assert_eq!([unpack(0), unpack(1), unpack(2)], [0x1234, 0xabcd, 0x0001]);
    assert_eq!(bytes.len(), 8);
}
