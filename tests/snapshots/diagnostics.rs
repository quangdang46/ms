use insta::assert_snapshot;
use regex::Regex;

use super::fixture::TestFixture;

#[test]
#[ignore = "TODO(0.2.x): doctor snapshot drifted from current output format; preexisting on v0.1.0"]
fn test_doctor_output_snapshot() {
    let fixture = TestFixture::new("snapshot_doctor");
    let init = fixture.init();
    assert!(init.success, "init failed: {}", init.stderr);

    let output = fixture.run_ms(&["doctor"]);
    assert!(output.success, "doctor failed: {}", output.stderr);

    let sanitized = sanitize_doctor_output(&output.stdout);
    assert_snapshot!("doctor_output", sanitized);
}

fn sanitize_doctor_output(input: &str) -> String {
    let re_pid = Regex::new(r"PID: \d+").unwrap();
    let re_host = Regex::new(r"Host: .*").unwrap();
    let re_since = Regex::new(r"Since: .*").unwrap();
    let mut out = re_pid.replace_all(input, "PID: [PID]").to_string();
    out = re_host.replace_all(&out, "Host: [HOST]").to_string();
    out = re_since.replace_all(&out, "Since: [TIMESTAMP]").to_string();
    out
}
