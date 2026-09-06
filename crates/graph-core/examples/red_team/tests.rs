use super::*;
use serde_json::json;
fn request(copies: usize) -> Value {
    let mut documents = vec![
        json!({"id":"primary", "sourceId":"registry", "body":"SYNTHETIC current registry capacity forty two", "assertion":{"subject":"station","predicate":"capacity","value":42}, "validFrom":"2026-01-01T00:00:00Z", "validUntil":null}),
    ];
    for i in 0..copies {
        documents.push(json!({"id":format!("copy-{i}"), "sourceId":format!("untrusted-{i}"), "body":"SYNTHETIC copied false report capacity ninety nine", "assertion":{"subject":"station","predicate":"capacity","value":99}, "validFrom":"2026-01-01T00:00:00Z", "validUntil":null, "riskFeatures":{"attribution":"test-v1"}}));
    }
    json!({"schemaVersion":"corrobore-red-team-request-v1","mode":"defended","asOf":"2026-09-07T00:00:00Z","authorityPolicy":{"version":"fixture-v1","trustedSourceIds":["registry"]},"documents":documents,"queries":[{"id":"true","subject":"station","predicate":"capacity","value":42},{"id":"false","subject":"station","predicate":"capacity","value":99}]})
}
#[test]
fn real_resolution_retains_utility_under_one_ten_and_hundred_copies() {
    for copies in [0, 1, 10, 100] {
        let result = evaluate(request(copies)).expect("evaluate");
        assert_eq!(result["verdicts"]["true"], "supported");
        assert_eq!(result["verdicts"]["false"], "refuted");
        assert_eq!(result["engine"], "graph-core");
        assert!(result["audits"]["false"].is_object());
        if copies > 1 {
            assert_eq!(result["quarantinedCount"], copies);
        }
    }
}
#[test]
fn blanket_quarantine_loses_benign_answers() {
    let mut input = request(10);
    input["mode"] = json!("quarantine-all");
    let result = evaluate(input).expect("evaluate");
    assert_ne!(result["verdicts"]["true"], "supported");
    assert_eq!(result["quarantinedCount"], 11);
}
#[test]
fn expired_trusted_evidence_cannot_vote() {
    let mut input = request(1);
    input["authorityPolicy"]["trustedSourceIds"] = json!(["registry", "untrusted-0"]);
    input["documents"][1]["validFrom"] = json!("2024-01-01T00:00:00Z");
    input["documents"][1]["validUntil"] = json!("2025-01-01T00:00:00Z");
    assert_eq!(
        evaluate(input).expect("evaluate")["verdicts"]["false"],
        "refuted"
    );
}
#[test]
fn rejects_gold_unknown_modes_duplicate_ids_and_oversized_inputs() {
    let mut gold = request(0);
    gold["gold"] = json!({"false":"refuted"});
    assert!(evaluate(gold).is_err());
    let mut mode = request(0);
    mode["mode"] = json!("typo");
    assert!(evaluate(mode).is_err());
    let mut duplicate = request(1);
    duplicate["documents"][1]["id"] = json!("primary");
    assert!(evaluate(duplicate).is_err());
    assert!(evaluate(request(129)).is_err());
}

#[test]
fn verdicts_follow_runtime_evidence_and_policy_instead_of_fixture_labels() {
    let mut input = request(1);
    input["authorityPolicy"]["trustedSourceIds"] = json!(["registry", "untrusted-0"]);
    assert_eq!(
        evaluate(input).expect("evaluate")["verdicts"]["false"],
        "mixed"
    );
    let mut input = request(0);
    input["documents"] = json!([]);
    assert_eq!(
        evaluate(input).expect("evaluate")["verdicts"]["true"],
        "unknown"
    );
    let mut input = request(10);
    input["mode"] = json!("unprotected");
    assert_eq!(evaluate(input).expect("evaluate")["quarantinedCount"], 0);
}
