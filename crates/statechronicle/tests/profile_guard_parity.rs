//! Immutable original-release characterization of public built-in validation.
#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "../../../ci/characterization/profile_guards.rs"]
mod corpus;

#[test]
fn profile_guards_match_the_original_release_trace() {
    let original: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/0.3/evidence/maintenance/profile-guards/original-manifest.json"
    ))
    .unwrap();
    let actual = corpus::transcript();
    let rows = actual.as_array().unwrap();
    assert_eq!(
        &serde_json::json!(rows.len()),
        original.get("case_count").unwrap()
    );
    assert_eq!(
        &serde_json::json!(
            rows.iter()
                .filter(|row| row.get("result").unwrap() == "Ok")
                .count()
        ),
        original.get("accepted").unwrap()
    );
    let digest = statechronicle::core::digest::hash_bytes(&serde_json::to_vec(&actual).unwrap());
    assert_eq!(
        digest.as_str(),
        original.get("transcript_digest").unwrap().as_str().unwrap()
    );
}
