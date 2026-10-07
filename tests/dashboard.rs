use fleet_sim::{dashboard, Engine};
use serde_json::Value;

#[test]
fn dashboard_static_assets_are_embedded() {
    let (index, index_type) = dashboard::static_asset("/").expect("index should exist");
    let (css, css_type) = dashboard::static_asset("/app.css").expect("css should exist");
    let (js, js_type) = dashboard::static_asset("/app.js").expect("js should exist");

    assert_eq!(index_type, "text/html");
    assert_eq!(css_type, "text/css");
    assert_eq!(js_type, "application/javascript");
    assert!(index.contains("Fleet Dashboard"));
    assert!(index.contains("/app.js"));
    assert!(css.contains("--committed"));
    assert!(js.contains("setActiveMode"));
    assert!(dashboard::static_asset("/missing").is_none());
}

#[test]
fn dashboard_command_body_returns_engine_json() {
    let mut engine = Engine::new();
    let body = r#"{"command":"init","parameters":{"travel_times":[[0,1],[1,0]],"dock_throughput":[2,2],"vessels":[{"id":"v1","location":0}],"cargo_manifest":[{"location":0,"destination":1,"quantity":2}]}}"#;

    let json = response_json(&dashboard::handle_command_body(&mut engine, body));

    assert_eq!(json["status"], "ok");
    assert_eq!(json["payload"]["tick"], 0);
    assert_eq!(json["payload"]["vessels"]["v1"]["state"], "idle");
    assert_eq!(json["payload"]["docks"]["0"]["cargo"][0]["destination"], 1);
    assert_eq!(json["payload"]["docks"]["0"]["cargo"][0]["quantity"], 2);
}

#[test]
fn dashboard_command_body_uses_simulator_error_json() {
    let mut engine = Engine::new();

    let json = response_json(&dashboard::handle_command_body(
        &mut engine,
        r#"{"command":"get_state","parameters":{}}"#,
    ));

    assert_eq!(json["status"], "error");
    assert_eq!(json["payload"]["message"], "engine is not initialized");
}

#[test]
fn dashboard_empty_command_body_returns_error_json() {
    let mut engine = Engine::new();

    let json = response_json(&dashboard::handle_command_body(&mut engine, "   "));

    assert_eq!(json["status"], "error");
    assert_eq!(json["payload"]["message"], "empty command");
}

fn response_json(response: &str) -> Value {
    serde_json::from_str(response).expect("response body should be json")
}
