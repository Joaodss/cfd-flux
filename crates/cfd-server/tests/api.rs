//! The REST API without jobs: capabilities, validation, errors, limits, CORS.

mod common;

use common::{client, small_channel, TestServer};
use reqwest::StatusCode;
use serde_json::{json, Value};

#[tokio::test]
async fn capabilities_list_the_engines_and_limits() {
    let s = TestServer::start("caps").await;
    let r = client()
        .get(s.url("/api/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let c: Value = r.json().await.unwrap();
    assert_eq!(c["methods"], json!(["lbm"]));
    let engines = c["engines"].as_array().unwrap();
    let kinds: Vec<&str> = engines
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["cuda", "wgpu", "cpu"]);
    assert_eq!(engines[2]["available"], true);
    assert_eq!(engines[2]["physicalDevice"], "cpu");
    assert!(engines[2]["threads"].as_u64().unwrap() >= 1);
    assert_eq!(engines[0]["available"], false);
    assert_eq!(engines[0]["reason"], "disabled");
    assert_eq!(c["auto"], "cpu");
    assert_eq!(c["limits"]["maxCells"], 4096 * 4096);
    assert_eq!(c["defaults"]["streamDtype"], "f16");
    s.stop().await;
}

#[tokio::test]
async fn a_valid_scene_gets_an_estimate() {
    let s = TestServer::start("validate-ok").await;
    let r = client()
        .post(s.url("/api/scenes/validate"))
        .json(&small_channel())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let e = &v["estimate"];
    assert_eq!(e["cells"], 64 * 18);
    assert_eq!(e["engine"], "cpu");
    assert_eq!(e["memoryBytes"], 64 * 18 * 94);
    assert!(e["steps"].as_u64().unwrap() > 0);
    assert!(e.get("seconds").is_none(), "no throughput measured yet");
    // The numbers derived from the scene, in physical and lattice units.
    let n = &e["numbers"];
    for key in [
        "velocity",
        "length",
        "reynolds",
        "mach",
        "tau",
        "dx",
        "dt",
        "latticeVelocity",
    ] {
        assert!(n[key].as_f64().unwrap() > 0.0, "{key}: {n}");
    }
    assert!(n.get("prandtl").is_none(), "isothermal");
    let re = n["velocity"].as_f64().unwrap() * n["length"].as_f64().unwrap() / 1.516e-5;
    assert!(
        (n["reynolds"].as_f64().unwrap() / re - 1.0).abs() < 1e-9,
        "{n}"
    );
    // Without a scene `reference`, the reference scales are the characteristic ones.
    let r = &n["reference"];
    assert_eq!(r["reynolds"], n["reynolds"], "{n}");
    assert_eq!(r["length"], n["length"], "{n}");
    s.stop().await;
}

#[tokio::test]
async fn issues_carry_params_and_cells() {
    let s = TestServer::start("validate-params").await;
    let mut scene = small_channel();
    let n = scene.grid.cell_count();
    let mut ct = scene.layers.cell_type.decode_u8(n).unwrap();
    ct[5 * 64 + 30] = 1; // a solid cell without an element at (30, 5)
    scene.layers.cell_type =
        cfd_core::layers::LayerData::encode_u8(&ct, cfd_core::layers::LayerEncoding::RawBase64);
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&scene)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let issue = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["code"] == "cell.element")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(issue["at"], serde_json::json!([30, 5]));
    assert_eq!(issue["params"]["problem"], "noElement");
    // Issues without values or a cell leave the fields out.
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&small_channel())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for i in v["issues"].as_array().unwrap() {
        assert!(i.get("at").is_none(), "{i}");
    }
    s.stop().await;
}

#[tokio::test]
async fn an_invalid_scene_is_reported_with_issue_codes() {
    let s = TestServer::start("validate-bad").await;
    let mut scene = small_channel();
    scene.grid.width = 65; // the layers no longer match the grid
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&scene)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["code"], "scene.invalid");
    assert!(v["estimate"].is_null());
    let codes: Vec<&str> = v["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"layer.cellType"), "{codes:?}");
    assert_eq!(v["issues"][0]["severity"], "error");

    // An engine the scene asks for that this server does not have.
    let mut cuda = small_channel();
    cuda.run.backend = cfd_core::scene::BackendPreference::Cuda;
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&cuda)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["code"], "engine.unavailable", "{v}");
    s.stop().await;
}

#[tokio::test]
async fn malformed_requests_get_a_json_error() {
    let s = TestServer::start("malformed").await;
    let r = client()
        .post(s.url("/api/scenes/validate"))
        .header("content-type", "application/json")
        .body("{ not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "request.json");

    // Valid JSON that is not a scene.
    let r = client()
        .post(s.url("/api/scenes/validate"))
        .json(&json!({ "hello": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "scene.format");

    let r = client().get(s.url("/api/nothing")).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "notFound");
    s.stop().await;
}

#[tokio::test]
async fn limits_are_enforced() {
    let s = TestServer::with("limits", &[cfd_engines::EngineKind::Cpu], |c| {
        c.limits.max_cells = 1000;
        c.limits.max_body_bytes = 512;
    })
    .await;
    // The small channel's JSON is over a 512-byte body limit.
    let body = serde_json::to_vec(&small_channel()).unwrap();
    assert!(body.len() > 512);
    let r = client()
        .post(s.url("/api/scenes/validate"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("zstd+base64"),
        "{v}"
    );
    s.stop().await;

    let s = TestServer::with("limits-cells", &[cfd_engines::EngineKind::Cpu], |c| {
        c.limits.max_cells = 1000;
    })
    .await;
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&small_channel())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["code"], "limits.cells", "{v}");
    // The estimate still says why (and what the scene would need).
    assert_eq!(v["estimate"]["cells"], 64 * 18, "{v}");
    assert!(v["estimate"]["engine"].is_null());
    s.stop().await;
}

#[tokio::test]
async fn the_vite_dev_server_may_call_the_api() {
    let s = TestServer::start("cors").await;
    let r = client()
        .request(reqwest::Method::OPTIONS, s.url("/api/scenes/validate"))
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    assert_eq!(
        r.headers()["access-control-allow-origin"],
        "http://localhost:5173"
    );
    let r = client()
        .get(s.url("/api/capabilities"))
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none());
    s.stop().await;
}
