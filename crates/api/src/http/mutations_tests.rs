use super::*;
use serde::ser::{Serialize, Serializer};

struct Unserializable;
impl Serialize for Unserializable {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("synthetic serialization failure"))
    }
}
fn saved(state: &str) -> Receipt {
    Receipt {
        request_hash: vec![],
        state: state.into(),
        target_id: Some(Uuid::now_v7()),
        voyage_id: Some(Uuid::now_v7()),
        result_revision: Some(8),
        previous_revision: Some(7),
        response_status: Some(200),
        response_body: Some(br#"{"status":"published"}"#.to_vec()),
        resource_kind: Some("publication".into()),
    }
}
#[test]
fn publication_and_rejection_summaries_are_historical_and_safe() {
    let receipt = saved("applied");
    let summary = receipt.summary();
    assert_eq!(summary["publication_status"], "published");
    assert_eq!(summary["previous_revision"], "7");
    assert_eq!(
        summary["voyage_id"],
        receipt.voyage_id.expect("voyage").to_string()
    );
    assert!(summary.get("published").is_none());
    for bytes in [
        None,
        Some(b"broken JSON".to_vec()),
        Some(br#"{"code":"conflict"}"#.to_vec()),
    ] {
        let mut receipt = saved("rejected");
        receipt.response_body = bytes;
        assert!(receipt.summary().get("message").is_none());
    }
    let mut receipt = saved("rejected");
    receipt.response_body =
        Some(br#"{"message":"Review this change","private_fields":"must not leak"}"#.to_vec());
    let summary = receipt.summary();
    assert_eq!(summary["message"], "Review this change");
    assert!(summary.get("private_fields").is_none());
    let mut receipt = saved("applied");
    receipt.response_body = Some(b"broken JSON".to_vec());
    assert!(receipt.summary().get("publication_status").is_none());
    receipt.response_body = None;
    assert!(receipt.summary().get("publication_status").is_none());
}
#[tokio::test]
async fn receipt_replay_handles_deleted_rejected_and_incomplete_outcomes() {
    let deleted = saved("deleted").replay().expect("deleted receipt");
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(deleted.headers()[header::CACHE_CONTROL], "no-store");
    for status in [None, Some(0), Some(-1)] {
        let mut receipt = saved("in_progress");
        receipt.response_status = status;
        assert!(matches!(
            receipt.replay(),
            Err(AppError::ServiceUnavailable)
        ));
    }
    let mut receipt = saved("rejected");
    receipt.response_status = Some(412);
    receipt.response_body = None;
    let replay = receipt.replay().expect("recorded rejection");
    assert_eq!(replay.status(), StatusCode::PRECONDITION_FAILED);
    assert!(
        to_bytes(replay.into_body(), 1024)
            .await
            .expect("body")
            .is_empty()
    );
    assert!(
        !response(StatusCode::NO_CONTENT, vec![])
            .headers()
            .contains_key(header::CONTENT_TYPE)
    );
}
#[test]
fn serialization_failures_propagate_and_empty_outcomes_preserve_resource_identity() {
    let id = Uuid::now_v7();
    assert!(matches!(
        Outcome::json(StatusCode::CREATED, id, id, "voyage", 1, Unserializable),
        Err(AppError::Internal(_))
    ));
    let outcome = Outcome::empty(StatusCode::NO_CONTENT, id, id, "block", 0);
    assert!(!outcome.deleted);
    assert_eq!(outcome.target_id, id);
    assert!(outcome.body.is_empty());
}
