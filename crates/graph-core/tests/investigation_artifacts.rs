#![allow(clippy::unwrap_used)]
//! An artifact is a view, not a fact: it binds records by identity, regenerates
//! from live ones, keeps every analyst note, and publishing decides nothing
//! about truth.
use graph_core::*;

const ARTIFACT: &str = "artifact--convoy-brief";
const CLAIM: &str = "claim--convoy-delay";

fn stamp(system: &str) -> BitemporalStamp {
    BitemporalStamp::new(
        TemporalTimestamp::new("2026-09-07T00:00:00Z").unwrap(),
        TemporalTimestamp::new(system).unwrap(),
    )
    .unwrap()
}

fn note(id: &str, text: &str) -> ArtifactAnnotation {
    ArtifactAnnotation::new(
        id,
        ActorId::new("analyst--fimi").unwrap(),
        text,
        stamp("2026-09-07T01:00:00Z"),
    )
    .unwrap()
}

fn binding(claims: &[&str], evidence: &[&str]) -> ArtifactBinding {
    ArtifactBinding {
        claims: claims.iter().map(|id| ClaimId::new(*id).unwrap()).collect(),
        evidence: evidence
            .iter()
            .map(|id| EvidenceId::new(*id).unwrap())
            .collect(),
        narratives: vec![],
        campaigns: vec![],
    }
}

fn fixture() -> Graph {
    let mut graph = Graph::new();
    graph
        .create_evidence(EvidenceInput::new(
            EvidenceId::new("evidence--reading").unwrap(),
            "https://registry.test/reading",
            "the convoy was held four hours",
        ))
        .unwrap();
    for id in [CLAIM, "claim--second-outlet"] {
        graph
            .epistemic_stores_mut()
            .claims
            .create_asserted_claim(ClaimInput::new(
                ClaimId::new(id).unwrap(),
                ClaimStatement::new("one aid convoy was held four hours").unwrap(),
                ClaimTarget::AnalyticalAssertion(ClaimAnalyticalTarget::new("convoy", None)),
            ))
            .unwrap();
    }
    graph
}

fn created(graph: &mut Graph) -> String {
    graph
        .create_artifact(
            ArtifactInput::new(
                ARTIFACT,
                ArtifactKind::AnalystBrief,
                "Checkpoint delay brief",
                binding(&[CLAIM], &["evidence--reading"]),
                stamp("2026-09-07T00:10:00Z"),
            )
            .with_annotation(note("annotation--context", "the release time matters")),
        )
        .unwrap()
}

//
// An artifact names records by identity and has nowhere to put factual text:
// its only free text is a title and an analyst note.
#[test]
fn an_artifact_binds_records_by_identity_and_carries_no_factual_text() {
    let mut graph = fixture();
    let id = created(&mut graph);
    let artifact = graph
        .epistemic_stores()
        .artifacts
        .artifact_by_id(&id)
        .unwrap();

    assert_eq!(artifact.kind(), ArtifactKind::AnalystBrief);
    assert_eq!(artifact.title(), "Checkpoint delay brief");
    assert_eq!(artifact.versions().len(), 1);
    let current = artifact.current();
    assert_eq!(current.version(), 1);
    assert_eq!(current.publication(), PublicationState::Draft);
    assert_eq!(current.binding().claims, vec![ClaimId::new(CLAIM).unwrap()]);
    assert_eq!(current.annotations().len(), 1);
    assert_eq!(current.annotations()[0].note(), "the release time matters");

    // Every factual reference in the serialized artifact is an identifier.
    let serialized = serde_json::to_string(artifact).unwrap();
    assert!(serialized.contains(CLAIM));
    assert!(
        !serialized.contains("the convoy was held four hours"),
        "an artifact never copies the evidence payload"
    );
    assert_eq!(ArtifactKind::ALL.len(), 5);
    assert_eq!(PublicationState::Withdrawn.as_str(), "withdrawn");
}

//
// An artifact that names no record is refused: it would have to be carrying its
// content some other way.
#[test]
fn an_artifact_that_binds_nothing_is_refused() {
    let mut graph = fixture();
    assert!(
        graph
            .create_artifact(ArtifactInput::new(
                "artifact--empty",
                ArtifactKind::Timeline,
                "Empty",
                ArtifactBinding::default(),
                stamp("2026-09-07T00:10:00Z"),
            ))
            .is_err()
    );
    assert!(
        graph
            .create_artifact(ArtifactInput::new(
                "artifact--blank-title",
                ArtifactKind::Timeline,
                "   ",
                binding(&[CLAIM], &[]),
                stamp("2026-09-07T00:10:00Z"),
            ))
            .is_err()
    );
    assert!(
        graph
            .create_artifact(ArtifactInput::new(
                "artifact--unknown-claim",
                ArtifactKind::Timeline,
                "Unknown",
                binding(&["claim--absent"], &[]),
                stamp("2026-09-07T00:10:00Z"),
            ))
            .is_err()
    );
    assert!(graph.epistemic_stores().artifacts.is_empty());
}

//
// Regeneration reads the records as they stand now and keeps every analyst
// note, and the earlier version stays exactly as it was.
#[test]
fn regeneration_refreshes_the_binding_and_preserves_every_annotation() {
    let mut graph = fixture();
    let id = created(&mut graph);
    graph
        .annotate_artifact(
            &id,
            note("annotation--review", "reviewed against the registry"),
            stamp("2026-09-07T02:00:00Z"),
        )
        .unwrap();
    let before = graph
        .epistemic_stores()
        .artifacts
        .artifact_by_id(&id)
        .unwrap()
        .current()
        .clone();

    let version = graph
        .regenerate_artifact(
            &id,
            binding(&[CLAIM, "claim--second-outlet"], &["evidence--reading"]),
            stamp("2026-09-07T03:00:00Z"),
        )
        .unwrap();
    let artifact = graph
        .epistemic_stores()
        .artifacts
        .artifact_by_id(&id)
        .unwrap();

    assert_eq!(version, 3);
    assert_eq!(artifact.current().binding().claims.len(), 2);
    assert_eq!(
        artifact
            .current()
            .annotations()
            .iter()
            .map(ArtifactAnnotation::id)
            .collect::<Vec<_>>(),
        ["annotation--context", "annotation--review"],
        "regeneration carries every analyst note forward"
    );
    assert_eq!(
        artifact.version(2).unwrap(),
        &before,
        "an earlier version is never rewritten"
    );
    assert_eq!(artifact.version(1).unwrap().binding().claims.len(), 1);
    assert!(artifact.version(9).is_none());
}

//
// Publishing is a permissions decision: it reads no verdict, changes no claim,
// and an artifact about a refuted claim publishes exactly like any other.
#[test]
fn publication_is_independent_of_evidence_and_truth_state() {
    let mut graph = fixture();
    let id = created(&mut graph);
    let claims = graph.epistemic_stores().claims.clone();
    let verdicts = graph.epistemic_stores().verdicts.clone();

    let published = graph
        .set_artifact_publication(
            &id,
            PublicationState::Published,
            stamp("2026-09-07T04:00:00Z"),
        )
        .unwrap();
    let withdrawn = graph
        .set_artifact_publication(
            &id,
            PublicationState::Withdrawn,
            stamp("2026-09-07T05:00:00Z"),
        )
        .unwrap();
    let artifact = graph
        .epistemic_stores()
        .artifacts
        .artifact_by_id(&id)
        .unwrap();

    assert_eq!((published, withdrawn), (2, 3));
    assert_eq!(
        artifact.version(2).unwrap().publication(),
        PublicationState::Published
    );
    assert_eq!(
        artifact.current().publication(),
        PublicationState::Withdrawn
    );
    assert_eq!(
        artifact.current().annotations().len(),
        1,
        "a publication change keeps the notes"
    );
    assert_eq!(
        graph.epistemic_stores().claims,
        claims,
        "publishing changes no claim"
    );
    assert_eq!(
        graph.epistemic_stores().verdicts,
        verdicts,
        "publishing reads and writes no verdict"
    );
}

//
// A brief keeps its lineage: creation is idempotent by content, a reused
// identity with different content is a conflict, and annotations are unique.
#[test]
fn artifact_lineage_is_append_only_and_identities_do_not_collide() {
    let mut graph = fixture();
    let id = created(&mut graph);
    let replay = created(&mut graph);
    assert_eq!(id, replay);
    assert_eq!(
        graph
            .epistemic_stores()
            .artifacts
            .artifact_by_id(&id)
            .unwrap()
            .versions()
            .len(),
        1,
        "an identical creation is idempotent"
    );

    assert!(
        graph
            .create_artifact(ArtifactInput::new(
                ARTIFACT,
                ArtifactKind::Timeline,
                "Different content",
                binding(&[CLAIM], &[]),
                stamp("2026-09-07T00:10:00Z"),
            ))
            .is_err()
    );
    assert!(
        graph
            .annotate_artifact(
                &id,
                note(
                    "annotation--context",
                    "a second note with the same identity"
                ),
                stamp("2026-09-07T06:00:00Z"),
            )
            .is_err()
    );
    assert!(
        graph
            .regenerate_artifact(
                "artifact--absent",
                binding(&[CLAIM], &[]),
                stamp("2026-09-07T06:00:00Z"),
            )
            .is_err()
    );
    assert!(
        ArtifactAnnotation::new(
            "annotation--blank",
            ActorId::new("analyst--fimi").unwrap(),
            "   ",
            stamp("2026-09-07T06:00:00Z"),
        )
        .is_err()
    );
}

//
// Artifacts are governed records: they survive a native round trip, restoration
// refuses a binding to a record that is gone, and a graph without artifacts
// exports the bytes it did before.
#[test]
fn artifacts_survive_a_round_trip_and_restoration_refuses_a_dangling_binding() {
    let bare = fixture().export_memory_json().unwrap();
    assert!(!bare.contains("artifacts"));

    let mut graph = fixture();
    let id = created(&mut graph);
    graph
        .set_artifact_publication(
            &id,
            PublicationState::Published,
            stamp("2026-09-07T04:00:00Z"),
        )
        .unwrap();
    let exported = graph.export_memory_json().unwrap();
    let restored = Graph::from_memory_json(&exported).unwrap();

    assert_eq!(restored.export_memory_json().unwrap(), exported);
    assert_eq!(
        restored
            .epistemic_stores()
            .artifacts
            .artifact_by_id(&id)
            .unwrap()
            .current()
            .publication(),
        PublicationState::Published
    );

    // Repoint only the artifact's binding, leaving the claim store intact, so
    // the snapshot carries an artifact naming a record that is not there.
    let mut snapshot: serde_json::Value = serde_json::from_str(&exported).unwrap();
    for version in snapshot["epistemic"]["artifacts"]["artifacts"][0]["versions"]
        .as_array_mut()
        .expect("artifact versions")
    {
        version["binding"]["claims"][0] = serde_json::json!({"value": "claim--vanished"});
    }
    assert!(
        Graph::from_memory_json(&snapshot.to_string()).is_err(),
        "an artifact cannot outlive the records it names"
    );
}
