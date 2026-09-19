use nx::{CheckRequest, DaemonRequest, DaemonResponse, ReportTarget};
use std::path::PathBuf;

#[test]
fn check_request_round_trips_through_json() {
    let request = DaemonRequest::Check(CheckRequest {
        flake: PathBuf::from("/etc/nixos"),
        host: "test-system".to_owned(),
        offline: true,
    });

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains(r#""command":"check""#));

    let decoded = match serde_json::from_str(&json).unwrap() {
        DaemonRequest::Check(decoded) => decoded,
        DaemonRequest::List(_) => panic!("decoded as list request"),
    };
    assert_eq!(decoded.flake, PathBuf::from("/etc/nixos"));
    assert_eq!(decoded.host, "test-system");
    assert!(decoded.offline);
}

#[test]
fn list_request_round_trips_through_json() {
    let request = DaemonRequest::List(ReportTarget {
        flake: PathBuf::from("/etc/nixos"),
        configuration: "test-system".to_owned(),
    });

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains(r#""command":"list""#));

    let decoded = match serde_json::from_str(&json).unwrap() {
        DaemonRequest::List(decoded) => decoded,
        DaemonRequest::Check(_) => panic!("decoded as check request"),
    };
    assert_eq!(decoded.flake, PathBuf::from("/etc/nixos"));
    assert_eq!(decoded.configuration, "test-system");
}

#[test]
fn failure_response_round_trips_through_json() {
    let json = serde_json::to_string(&DaemonResponse::failure("evaluation failed")).unwrap();
    let decoded: DaemonResponse = serde_json::from_str(&json).unwrap();

    assert!(!decoded.ok);
    assert!(decoded.report.is_none());
    assert_eq!(decoded.error.as_deref(), Some("evaluation failed"));
}
