use std::path::PathBuf;

use cfd_core::examples;
use cfd_core::layers::{LayerData, LayerEncoding};
use cfd_core::scene::{CellType, Scene};
use cfd_core::validate::validate;

fn repo_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn read_normalized(rel: &str) -> String {
    let path = repo_file(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

fn error_codes(scene: &Scene) -> Vec<&'static str> {
    validate(scene).errors().map(|i| i.code).collect()
}

#[test]
fn examples_are_valid() {
    for name in examples::NAMES {
        let scene = examples::by_name(name).unwrap();
        let report = validate(&scene);
        assert!(report.is_ok(), "{name}: {:#?}", report.issues);
    }
}

#[test]
fn examples_round_trip_through_json() {
    for name in examples::NAMES {
        let scene = examples::by_name(name).unwrap();
        let parsed = Scene::from_json(&scene.to_json_pretty()).unwrap();
        assert_eq!(parsed, scene, "{name}");
    }
}

#[test]
fn committed_examples_are_up_to_date() {
    for name in examples::NAMES {
        let generated = examples::by_name(name).unwrap().to_json_pretty() + "\n";
        let committed = read_normalized(&format!("scenes/{name}.json"));
        assert!(
            generated == committed,
            "scenes/{name}.json is outdated; run `cargo run -p cfd-cli -- example --all scenes`"
        );
    }
}

#[test]
fn committed_schema_is_up_to_date() {
    let generated = serde_json::to_string_pretty(&Scene::json_schema()).unwrap() + "\n";
    let committed = read_normalized("schema/scene.schema.json");
    assert!(
        generated == committed,
        "schema/scene.schema.json is outdated; run `cargo run -p cfd-cli -- schema schema/scene.schema.json`"
    );
}

#[test]
fn unknown_fields_are_rejected() {
    let mut json: serde_json::Value =
        serde_json::from_str(&examples::channel().to_json_pretty()).unwrap();
    json["grid"]["colour"] = "red".into();
    assert!(serde_json::from_value::<Scene>(json).is_err());
}

/// Replaces the cellType/elementId of one cell in a scene.
fn paint(scene: &mut Scene, x: u32, y: u32, ct: u8, element: u16) {
    let n = scene.grid.cell_count();
    let i = (y * scene.grid.width + x) as usize;
    let mut cell_type = scene.layers.cell_type.decode_u8(n).unwrap();
    let mut element_id = scene.layers.element_id.decode_u16(n).unwrap();
    cell_type[i] = ct;
    element_id[i] = element;
    scene.layers.cell_type = LayerData::encode_u8(&cell_type, LayerEncoding::RawBase64);
    scene.layers.element_id = LayerData::encode_u16(&element_id, LayerEncoding::RawBase64);
}

#[test]
fn detects_cell_element_mismatches() {
    // Solid cell without element.
    let mut s = examples::channel();
    paint(&mut s, 200, 50, CellType::Solid as u8, 0);
    assert!(error_codes(&s).contains(&"cell.element"));

    // Solid cell pointing at the inlet element.
    let mut s = examples::channel();
    paint(&mut s, 200, 50, CellType::Solid as u8, 2);
    assert!(error_codes(&s).contains(&"cell.element"));

    // Unknown cell type.
    let mut s = examples::channel();
    paint(&mut s, 200, 50, 42, 0);
    assert!(error_codes(&s).contains(&"cell.type"));

    // Outlet cell in the bottom wall, with the fluid cell above it also turned into wall.
    let mut s = examples::channel();
    paint(&mut s, 200, 1, CellType::Solid as u8, 1);
    paint(&mut s, 200, 0, CellType::Outlet as u8, 3);
    assert!(error_codes(&s).contains(&"cell.element"));

    // Empty cell without free surface.
    let mut s = examples::channel();
    paint(&mut s, 200, 50, CellType::Empty as u8, 0);
    assert!(error_codes(&s).contains(&"cell.empty"));
}

#[test]
fn detects_bad_references_and_sizes() {
    let mut s = examples::channel();
    s.initial.fluid = 9;
    assert!(error_codes(&s).contains(&"initial.fluid"));

    let mut s = examples::channel();
    s.grid.width += 1;
    assert!(error_codes(&s).contains(&"layer.cellType"));

    let mut s = examples::channel();
    s.elements[1].id = 1;
    assert!(error_codes(&s).contains(&"id.duplicate"));

    let mut s = examples::channel();
    s.probes[0].position = [10_000, 0];
    assert!(error_codes(&s).contains(&"probe.position"));
}
