use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn run_fixture(input: &str) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fleet-sim"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn fleet-sim");

    {
        let mut stdin = child.stdin.take().expect("stdin should be piped");
        stdin
            .write_all(input.as_bytes())
            .expect("failed to write fixture to stdin");
    }

    let output = child.wait_with_output().expect("failed to read output");
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "stderr should be empty, got {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("stdout should be utf-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line should be valid json"))
        .collect()
}

fn expected_fixture(expected: &str) -> Vec<Value> {
    expected
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("fixture line should be valid json"))
        .collect()
}

#[test]
fn base_flow_fixture_matches_expected_json() {
    let actual = run_fixture(include_str!("../examples/base.input.jsonl"));
    let expected = expected_fixture(include_str!("../examples/base.expected.jsonl"));

    assert_eq!(actual, expected);
}

#[test]
fn edge_case_fixture_matches_expected_json() {
    let actual = run_fixture(include_str!("../examples/edge_cases.input.jsonl"));
    let expected = expected_fixture(include_str!("../examples/edge_cases.expected.jsonl"));

    assert_eq!(actual, expected);
}

#[test]
fn follow_on_1_fixture_matches_expected_json() {
    let actual = run_fixture(include_str!("../examples/follow_on_1.input.jsonl"));
    let expected = expected_fixture(include_str!("../examples/follow_on_1.expected.jsonl"));

    assert_eq!(actual, expected);
}
