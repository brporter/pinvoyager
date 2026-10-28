//! Shared captured-response comparison; ordinary tests never regenerate files.
#![allow(dead_code)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("web")
        .join("src")
        .join("api")
        .join("__fixtures__")
}

/// Validates the captured body as JSON and compares it with the committed
/// fixture. With PINVOYAGER_UPDATE_FIXTURES=1, writes it with a trailing newline.
pub fn write_fixture(name: &str, body: &str) {
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("the fixtures directory should be creatable");

    // Parsed only to refuse writing something that is not JSON. A fixture that
    // is an HTML error page would fail the TypeScript import with a message
    // about modules rather than about drift, which is a bad way to find out.
    serde_json::from_str::<Value>(body)
        .unwrap_or_else(|err| panic!("the body captured for {name} is not JSON ({err}): {body}"));

    let path = dir.join(format!("{name}.json"));
    if std::env::var_os("PINVOYAGER_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, format!("{body}\n")).expect("write requested fixture update");
    } else {
        let committed = std::fs::read_to_string(&path).expect("committed fixture");
        let actual: Value = serde_json::from_str(body).expect("response JSON");
        let expected: Value = serde_json::from_str(&committed).expect("fixture JSON");
        assert_eq!(
            normalized(actual),
            normalized(expected),
            "fixture {name} changed; review and regenerate explicitly"
        );
    }
}

// Normalize only generated identities and instants; shape and functional values
// must still agree. URL paths and compound pagination cursors contain both.
fn normalized(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, normalized(value)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(normalized).collect()),
        Value::String(text) => Value::String(
            text.split('|')
                .map(|part| {
                    if chrono::DateTime::parse_from_rfc3339(part).is_ok() {
                        return "2000-01-01T00:00:00Z".to_owned();
                    }
                    part.split('/')
                        .map(|segment| {
                            if Uuid::parse_str(segment).is_ok() {
                                "00000000-0000-0000-0000-000000000001"
                            } else {
                                segment
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .collect::<Vec<_>>()
                .join("|"),
        ),
        other => other,
    }
}
