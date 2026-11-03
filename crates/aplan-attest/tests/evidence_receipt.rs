use aplan_attest::ToolIdentity;
use aplan_attest::receipt::{
    DSSE_PAYLOAD_TYPE, MAX_RECEIPT_BYTES, PREDICATE_TYPE, ProducerIdentity, ProducerPrivilege,
    ReceiptError, ReceiptExpectation, ReceiptInput, ReceiptTransport, RepositoryContext,
    TestEvidenceStatement, TrustPolicy, build, parse_and_authenticate, parse_and_verify,
    provider_identity, repository_context, write_atomic,
};
use aplan_config::TestRunner;
use aplan_evidence::{
    TestBatchOutcome, TestBatchResultEntry, TestCatalogueRunner, TestCatalogueRunnerState,
    TestExecutionObservation, TestMaterialIdentity,
};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

fn runner() -> TestRunner {
    TestRunner {
        weight: None,
        exclusive: None,
        name: Some("unit".into()),
        dir: None,
        covers: None,
        covers_by_test: None,
        inputs_by_test: None,
        skipped: None,
        output_lines: None,
        max_output: None,
        expect_tests: None,
        max_memory: None,
        max_processes: None,
        wrap: None,
        run_selects: Some(true),
        list: vec!["runner".into(), "--list".into()],
        run: vec!["runner".into(), "--exact".into(), "{}".into()],
        batch: None,
        timeout_seconds: None,
    }
}

fn producer(privilege: ProducerPrivilege) -> ProducerIdentity {
    ProducerIdentity {
        repository: "owner/repository".into(),
        workflow: "checks.yml:unit".into(),
        run_id: "run-17".into(),
        event: "push".into(),
        privilege,
    }
}

fn input() -> ReceiptInput {
    let inputs_sha256 = "1".repeat(64);
    let environment_sha256 = "2".repeat(64);
    let toolchain_sha256 = "3".repeat(64);
    let mut composed = Sha256::new();
    composed.update(b"aplan-test-material-v1\0");
    for value in [&inputs_sha256, &environment_sha256, &toolchain_sha256] {
        for pair in value.as_bytes().as_chunks::<2>().0 {
            composed.update([
                u8::from_str_radix(std::str::from_utf8(pair).expect("hex"), 16).expect("hex"),
            ]);
        }
    }
    let digest = composed.finalize();
    let material = TestMaterialIdentity {
        inputs_sha256,
        environment_sha256,
        toolchain_sha256,
        sha256: digest
            .iter()
            .fold(String::with_capacity(64), |mut text, byte| {
                use std::fmt::Write as _;
                let _ = write!(text, "{byte:02x}");
                text
            }),
    };
    ReceiptInput {
        repository: RepositoryContext {
            commit: "5".repeat(40),
            tree_sha256: "6".repeat(64),
            modified: 0,
            untracked: 0,
            status_sha256: "7".repeat(64),
        },
        runner: runner(),
        observation: TestExecutionObservation {
            runner: TestCatalogueRunner {
                index: 1,
                label: "unit".into(),
                state: TestCatalogueRunnerState::Listed,
                names: vec!["suite::passes".into()],
            },
            requested: vec!["suite::passes".into()],
            outcomes: vec![TestBatchResultEntry {
                test: "suite::passes".into(),
                outcome: TestBatchOutcome::Passed,
                attempts: None,
                diagnostic: None,
            }],
            complete: true,
        },
        material,
        tool: ToolIdentity {
            version: "0.5.0-dev".into(),
            source: "8".repeat(40),
            facts_sha256: "9".repeat(64),
            binary_sha256: "a".repeat(64),
        },
        producer: producer(ProducerPrivilege::Trusted),
        started_at_epoch_seconds: 10,
        completed_at_epoch_seconds: 11,
    }
}

fn expectation(statement: &TestEvidenceStatement) -> ReceiptExpectation {
    ReceiptExpectation {
        provider_sha256: statement.predicate.provider.sha256.clone(),
        inputs_sha256: statement.predicate.material.inputs_sha256.clone(),
        environment_sha256: statement.predicate.material.environment_sha256.clone(),
        toolchain_sha256: statement.predicate.material.toolchain_sha256.clone(),
        material_sha256: statement.predicate.material.sha256.clone(),
        tool: statement.predicate.tool.clone(),
        repository_commit: Some(statement.predicate.repository.commit.clone()),
        allow_source_revision_movement: false,
    }
}

fn verify_local(
    statement: &TestEvidenceStatement,
    expected: &ReceiptExpectation,
) -> Result<TestEvidenceStatement, aplan_attest::receipt::ReceiptError> {
    parse_and_verify(
        &statement.canonical_json().expect("canonical statement"),
        expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
}

fn dsse(payload: &[u8], signature: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "payloadType": DSSE_PAYLOAD_TYPE,
        "payload": base64::engine::general_purpose::STANDARD.encode(payload),
        "signatures": [{"keyid": "test", "sig": signature}]
    }))
    .expect("envelope")
}

/// Proof for `evidence.portable_provider_receipts`.
#[test]
fn one_canonical_statement_round_trips_without_a_plan_identity() {
    let statement = build(input()).expect("complete observation builds");
    let bytes = statement.canonical_json().expect("canonical JSON");
    assert_eq!(statement.predicate_type, PREDICATE_TYPE);
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("statement JSON");
    assert!(value["predicate"].get("plan").is_none());
    assert_eq!(bytes, statement.canonical_json().expect("same bytes"));

    let verified = parse_and_verify(
        &bytes,
        &expectation(&statement),
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect("round trip verifies");
    assert_eq!(verified, statement);
}

#[test]
fn authenticated_prior_parse_keeps_trust_and_derived_identity_checks() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    assert_eq!(
        parse_and_authenticate(
            &bytes,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile
        )
        .expect("authenticated prior"),
        statement
    );

    let error = parse_and_authenticate(
        &bytes,
        &TrustPolicy::SameWorkflow {
            producer: producer(ProducerPrivilege::Trusted),
        },
        &ReceiptTransport::SameWorkflow {
            artifact_sha256: "0".repeat(64),
            producer: producer(ProducerPrivilege::Trusted),
        },
    )
    .expect_err("a prior receipt still needs transport authentication");
    assert!(matches!(error, ReceiptError::Trust(_)), "{error}");

    let mut forged = statement;
    forged.predicate.provider.sha256 = "0".repeat(64);
    let error = parse_and_authenticate(
        &forged.canonical_json().expect("forged canonical bytes"),
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("derived identities are rechecked before reuse");
    let rendered = error.to_string();
    assert!(
        rendered.contains("provider") && rendered.contains("digest"),
        "{rendered}"
    );
}

#[test]
fn typed_statements_and_derived_subjects_obey_the_portable_document_bound() {
    let mut statement = build(input()).expect("statement");
    statement.predicate.provider.label = "x".repeat(MAX_RECEIPT_BYTES);
    let error = statement
        .canonical_json()
        .expect_err("typed statement exceeds the receipt bound");
    assert!(error.to_string().contains("exceeds"), "{error}");

    let mut value = input();
    value.observation.runner.label = "x".repeat(MAX_RECEIPT_BYTES);
    let error = build(value).expect_err("derived subject exceeds the receipt bound");
    assert!(error.to_string().contains("exceeds"), "{error}");
}

#[test]
fn a_forged_subject_and_a_valid_envelope_over_the_wrong_predicate_are_refused() {
    let statement = build(input()).expect("statement");
    let mut forged = serde_json::to_value(&statement).expect("value");
    forged["subject"][0]["digest"]["sha256"] = serde_json::json!("0".repeat(64));
    let forged = serde_json::to_vec(&forged).expect("bytes");
    let error = parse_and_verify(
        &forged,
        &expectation(&statement),
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("subject is derived, not trusted");
    assert!(error.to_string().contains("subject"), "{error}");

    let mut wrong: serde_json::Value =
        serde_json::from_slice(&statement.canonical_json().expect("canonical")).expect("value");
    wrong["predicateType"] = serde_json::json!("https://example.invalid/wrong/v1");
    let payload = serde_json::to_vec(&wrong).expect("payload");
    let envelope = serde_json::json!({
        "payloadType": "application/vnd.in-toto+json",
        "payload": base64::engine::general_purpose::STANDARD.encode(&payload),
        "signatures": [{"keyid": "test", "sig": "AA=="}]
    });
    let envelope = serde_json::to_vec(&envelope).expect("envelope");
    let transport = ReceiptTransport::Signed {
        payload_sha256: aplan_attest::sha256_hex(&payload),
        producer: producer(ProducerPrivilege::Trusted),
    };
    let error = parse_and_verify(
        &envelope,
        &expectation(&statement),
        &TrustPolicy::Signed {
            producer: producer(ProducerPrivilege::Trusted),
        },
        &transport,
    )
    .expect_err("authentication cannot make the wrong predicate relevant");
    assert!(error.to_string().contains("predicate"), "{error}");
}

#[test]
fn transport_integrity_and_producer_authentication_are_separate() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    let expectation = expectation(&statement);
    let trusted = producer(ProducerPrivilege::Trusted);
    let untrusted = producer(ProducerPrivilege::Untrusted);

    let wrong_digest = ReceiptTransport::SameWorkflow {
        artifact_sha256: "f".repeat(64),
        producer: trusted.clone(),
    };
    let error = parse_and_verify(
        &bytes,
        &expectation,
        &TrustPolicy::SameWorkflow {
            producer: trusted.clone(),
        },
        &wrong_digest,
    )
    .expect_err("content integrity is checked");
    assert!(matches!(error, ReceiptError::Trust(_)), "{error}");
    assert!(error.to_string().contains("artifact digest"), "{error}");

    let untrusted_transport = ReceiptTransport::SameWorkflow {
        artifact_sha256: aplan_attest::sha256_hex(&bytes),
        producer: untrusted,
    };
    let error = parse_and_verify(
        &bytes,
        &expectation,
        &TrustPolicy::SameWorkflow { producer: trusted },
        &untrusted_transport,
    )
    .expect_err("an untrusted producer cannot satisfy privileged policy");
    assert!(error.to_string().contains("producer"), "{error}");
}

#[test]
fn changed_material_is_stale_even_when_revision_movement_is_allowed() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    let mut expected = expectation(&statement);
    expected.allow_source_revision_movement = true;
    expected.repository_commit = Some("b".repeat(40));
    parse_and_verify(
        &bytes,
        &expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect("revision movement alone is audit context, not staleness");

    expected.inputs_sha256 = "c".repeat(64);
    let error = parse_and_verify(
        &bytes,
        &expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("revision movement does not waive material identity");
    assert!(error.to_string().contains("input material"), "{error}");
}

#[test]
fn a_required_current_revision_cannot_be_omitted_from_expectations() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    let mut expected = expectation(&statement);
    expected.repository_commit = None;
    let error = parse_and_verify(
        &bytes,
        &expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("missing expected identity fails closed");
    assert!(
        error.to_string().contains("repository commit is absent"),
        "{error}"
    );
}

#[test]
fn duplicate_outcomes_and_retry_created_unconditional_passes_are_refused() {
    let mut duplicate = input();
    duplicate
        .observation
        .outcomes
        .push(duplicate.observation.outcomes[0].clone());
    let error = build(duplicate).expect_err("duplicates are ambiguous");
    assert!(error.to_string().contains("repeats"), "{error}");

    let mut retry = input();
    retry.observation.outcomes[0].attempts = Some(2);
    let error = build(retry).expect_err("plain pass cannot hide retries");
    assert!(error.to_string().contains("retried_passed"), "{error}");

    let mut absent = input();
    absent.observation.requested = vec!["suite::absent".into()];
    absent.observation.outcomes[0].test = "suite::absent".into();
    let error = build(absent).expect_err("a request outside the catalogue is incomplete");
    assert!(
        error
            .to_string()
            .contains("absent from the complete catalogue")
    );

    let mut unordered = input();
    unordered.observation.runner.names = vec!["suite::a".into(), "suite::z".into()];
    unordered.observation.requested = vec!["suite::a".into(), "suite::z".into()];
    unordered.observation.outcomes = vec![
        TestBatchResultEntry {
            test: "suite::z".into(),
            outcome: TestBatchOutcome::Passed,
            attempts: None,
            diagnostic: None,
        },
        TestBatchResultEntry {
            test: "suite::a".into(),
            outcome: TestBatchOutcome::Passed,
            attempts: None,
            diagnostic: None,
        },
    ];
    let error = build(unordered).expect_err("outcomes have one canonical order");
    assert!(error.to_string().contains("strictly sorted"), "{error}");
}

#[test]
fn schema_is_derived_from_the_statement_type() {
    let schema = aplan_attest::receipt::derived_schema();
    let object = schema.as_object().expect("schema object");
    assert_eq!(
        object.get("title"),
        Some(&serde_json::json!("TestEvidenceStatement"))
    );
    let rendered = serde_json::to_string(&schema).expect("schema JSON");
    assert!(rendered.contains("additionalProperties"));
    assert!(rendered.contains("retried_passed"));
    assert!(rendered.contains("producer"));
}

#[test]
fn valid_same_workflow_and_dsse_transports_verify_the_same_statement() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    let expected = expectation(&statement);
    let authenticated = producer(ProducerPrivilege::Trusted);
    let same_workflow = parse_and_verify(
        &bytes,
        &expected,
        &TrustPolicy::SameWorkflow {
            producer: authenticated.clone(),
        },
        &ReceiptTransport::SameWorkflow {
            artifact_sha256: aplan_attest::sha256_hex(&bytes),
            producer: authenticated.clone(),
        },
    )
    .expect("authenticated artifact");
    assert_eq!(same_workflow, statement);

    let envelope = serde_json::to_vec(&serde_json::json!({
        "payloadType": "application/vnd.in-toto+json",
        "payload": base64::engine::general_purpose::STANDARD.encode(&bytes),
        "signatures": [{"keyid": "workflow-identity", "sig": "AA=="}]
    }))
    .expect("envelope");
    let signed = parse_and_verify(
        &envelope,
        &expected,
        &TrustPolicy::Signed {
            producer: authenticated.clone(),
        },
        &ReceiptTransport::Signed {
            payload_sha256: aplan_attest::sha256_hex(&bytes),
            producer: authenticated,
        },
    )
    .expect("externally authenticated envelope");
    assert_eq!(signed, statement);

    let url_safe_envelope = serde_json::to_vec(&serde_json::json!({
        "payloadType": DSSE_PAYLOAD_TYPE,
        "payload": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes),
        "signatures": [{"keyid": "", "sig": "_w"}]
    }))
    .expect("URL-safe envelope");
    let url_safe = parse_and_verify(
        &url_safe_envelope,
        &expected,
        &TrustPolicy::Signed {
            producer: producer(ProducerPrivilege::Trusted),
        },
        &ReceiptTransport::Signed {
            payload_sha256: aplan_attest::sha256_hex(&bytes),
            producer: producer(ProducerPrivilege::Trusted),
        },
    )
    .expect("unpadded URL-safe DSSE fields");
    assert_eq!(url_safe, statement);
}

#[test]
fn partial_truncated_unknown_and_noncanonical_documents_are_refused() {
    let mut partial = input();
    partial.observation.complete = false;
    assert!(
        build(partial)
            .expect_err("partial")
            .to_string()
            .contains("incomplete")
    );

    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    let expected = expectation(&statement);
    let truncated = &bytes[..bytes.len() / 2];
    assert!(
        parse_and_verify(
            truncated,
            &expected,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile,
        )
        .is_err()
    );

    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).expect("value");
    unknown["predicate"]["schema"] = serde_json::json!("aplan/test-evidence/v2");
    let unknown = serde_json::to_vec(&unknown).expect("unknown");
    let error = parse_and_verify(
        &unknown,
        &expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("unknown schema");
    assert!(
        error.to_string().contains("unknown predicate schema"),
        "{error}"
    );

    let pretty = serde_json::to_vec_pretty(&statement).expect("pretty");
    let error = parse_and_verify(
        &pretty,
        &expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("only canonical statement bytes are portable");
    assert!(error.to_string().contains("canonical JSON"), "{error}");
}

#[test]
fn runner_toolchain_tool_and_replayed_producer_drift_are_independent_refusals() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");

    let mut changed_runner = runner();
    changed_runner.run.push("--new-policy".into());
    let changed_provider =
        provider_identity(1, "unit", &changed_runner).expect("current provider identity");
    let mut changed_provider_expected = expectation(&statement);
    changed_provider_expected.provider_sha256 = changed_provider.sha256;
    let error = parse_and_verify(
        &bytes,
        &changed_provider_expected,
        &TrustPolicy::LocalFile,
        &ReceiptTransport::LocalFile,
    )
    .expect_err("current provider definition drift");
    assert!(error.to_string().contains("provider"), "{error}");

    for (name, mutate) in [
        ("provider", 0usize),
        ("toolchain", 1usize),
        ("tool", 2usize),
    ] {
        let mut expected = expectation(&statement);
        match mutate {
            0 => expected.provider_sha256 = "b".repeat(64),
            1 => expected.toolchain_sha256 = "c".repeat(64),
            2 => expected.tool.binary_sha256 = "d".repeat(64),
            _ => unreachable!(),
        }
        let error = parse_and_verify(
            &bytes,
            &expected,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile,
        )
        .expect_err("identity drift");
        assert!(error.to_string().contains(name), "{name}: {error}");
    }

    let wanted = producer(ProducerPrivilege::Trusted);
    let mut replayed = wanted.clone();
    replayed.run_id = "older-run".into();
    let error = parse_and_verify(
        &bytes,
        &expectation(&statement),
        &TrustPolicy::SameWorkflow { producer: wanted },
        &ReceiptTransport::SameWorkflow {
            artifact_sha256: aplan_attest::sha256_hex(&bytes),
            producer: replayed,
        },
    )
    .expect_err("a digest cannot authenticate the producer run");
    assert!(error.to_string().contains("producer"), "{error}");
}

#[test]
fn hostile_diagnostics_are_bounded_redacted_and_terminal_safe() {
    let mut value = input();
    value.observation.outcomes[0].outcome = TestBatchOutcome::Failed;
    value.observation.outcomes[0].diagnostic =
        Some("Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456\u{1b}[2J".into());
    let statement = build(value).expect("sanitized statement");
    let diagnostic = statement.predicate.outcomes[0]
        .diagnostic
        .as_deref()
        .expect("diagnostic");
    assert!(!diagnostic.contains("ghp_"), "{diagnostic}");
    assert!(!diagnostic.contains('\u{1b}'), "{diagnostic}");
    assert!(diagnostic.contains("[redacted]"), "{diagnostic}");

    let mut hostile_identity = input();
    hostile_identity.producer.workflow = "checks\u{1b}[2J".into();
    let error = build(hostile_identity).expect_err("identity controls are refused");
    let rendered = error.to_string();
    assert!(rendered.contains("unsafe terminal"), "{rendered}");
    assert!(!rendered.contains('\u{1b}'), "{rendered:?}");

    let mut oversized = input();
    oversized.observation.outcomes[0].outcome = TestBatchOutcome::Failed;
    oversized.observation.outcomes[0].diagnostic = Some("x".repeat(70 * 1024));
    let error = build(oversized).expect_err("oversized diagnostics are refused");
    assert!(error.to_string().contains("exceeds"), "{error}");
}

#[test]
fn construction_rejects_invalid_time_provider_and_catalogue_boundaries() {
    producer(ProducerPrivilege::Trusted)
        .validate()
        .expect("well-formed producer");
    let mut blank_producer = producer(ProducerPrivilege::Trusted);
    blank_producer.event = " ".into();
    assert!(blank_producer.validate().is_err());

    let mut backwards = input();
    backwards.completed_at_epoch_seconds = 9;
    assert!(
        build(backwards)
            .expect_err("backwards timestamps")
            .to_string()
            .contains("timestamp")
    );

    let mut zero = input();
    zero.observation.runner.index = 0;
    assert!(
        build(zero)
            .expect_err("zero provider")
            .to_string()
            .contains("provider")
    );
    assert!(provider_identity(0, "unit", &runner()).is_err());
    assert!(provider_identity(1, " ", &runner()).is_err());

    let mut unavailable = input();
    unavailable.observation.runner.state = TestCatalogueRunnerState::Failed;
    assert!(build(unavailable).is_err());

    let mut empty_catalogue = input();
    empty_catalogue.observation.runner.names.clear();
    empty_catalogue.observation.requested.clear();
    empty_catalogue.observation.outcomes.clear();
    assert!(build(empty_catalogue).is_err());

    let mut duplicate_catalogue = input();
    duplicate_catalogue
        .observation
        .runner
        .names
        .push("suite::passes".into());
    assert!(build(duplicate_catalogue).is_err());

    let mut empty_request = input();
    empty_request.observation.requested.clear();
    empty_request.observation.outcomes.clear();
    assert!(build(empty_request).is_err());

    let mut invalid_material = input();
    invalid_material.material.inputs_sha256 = "invalid".into();
    assert!(build(invalid_material).is_err());
    let mut invalid_tool = input();
    invalid_tool.tool.version = " ".into();
    assert!(build(invalid_tool).is_err());
    let mut invalid_repository = input();
    invalid_repository.repository.commit = "invalid".into();
    assert!(build(invalid_repository).is_err());
    let mut nonhex_repository = input();
    nonhex_repository.repository.commit = "z".repeat(40);
    assert!(build(nonhex_repository).is_err());
    let mut source_unknown = input();
    source_unknown.tool.source.clear();
    assert!(build(source_unknown).is_ok());
    let mut invalid_outcome = input();
    invalid_outcome.observation.outcomes[0].test = " ".into();
    assert!(build(invalid_outcome).is_err());
    let mut zero_attempts = input();
    zero_attempts.observation.outcomes[0].outcome = TestBatchOutcome::Failed;
    zero_attempts.observation.outcomes[0].attempts = Some(0);
    assert!(build(zero_attempts).is_err());
    let mut invalid_catalogue = input();
    invalid_catalogue.observation.runner.names[0] = " ".into();
    assert!(build(invalid_catalogue).is_err());
}

#[test]
fn imported_statements_recompute_every_derived_identity_and_execution_set() {
    let valid = build(input()).expect("statement");
    let expected = expectation(&valid);

    let mut cases: Vec<(&str, TestEvidenceStatement)> = Vec::new();
    let mut statement = valid.clone();
    statement.statement_type = "https://in-toto.io/Statement/v0.1".into();
    cases.push(("statement type", statement));
    let mut statement = valid.clone();
    statement.predicate.complete = false;
    cases.push(("partial", statement));
    let mut statement = valid.clone();
    statement.predicate.catalogue.complete = false;
    cases.push(("partial catalogue", statement));
    let mut statement = valid.clone();
    statement.predicate.provider.index = 0;
    cases.push(("provider index", statement));
    let mut statement = valid.clone();
    statement.predicate.provider.label = " ".into();
    cases.push(("provider label", statement));
    let mut statement = valid.clone();
    statement.predicate.provider.sha256 = "0".repeat(64);
    cases.push(("provider digest", statement));
    let mut statement = valid.clone();
    statement.predicate.provider.sha256 = "A".repeat(64);
    cases.push(("provider digest format", statement));
    let mut statement = valid.clone();
    statement.predicate.catalogue.names = vec!["z".into(), "a".into()];
    cases.push(("catalogue order", statement));
    let mut statement = valid.clone();
    statement.predicate.catalogue.sha256 = "0".repeat(64);
    cases.push(("catalogue digest", statement));
    let mut statement = valid.clone();
    statement.predicate.catalogue.sha256 = "A".repeat(64);
    cases.push(("catalogue digest format", statement));
    let mut statement = valid.clone();
    statement.predicate.requested.clear();
    cases.push(("empty request", statement));
    let mut statement = valid.clone();
    statement.predicate.requested = vec!["suite::passes".into(), "suite::passes".into()];
    cases.push(("duplicate request", statement));
    let mut statement = valid.clone();
    statement.predicate.outcomes.clear();
    cases.push(("missing outcome", statement));
    let mut statement = valid.clone();
    statement.predicate.outcomes[0].test = "suite::other".into();
    cases.push(("unrequested outcome", statement));
    let mut statement = valid.clone();
    statement.predicate.completed_at_epoch_seconds = 9;
    cases.push(("timestamp", statement));
    let mut statement = valid.clone();
    statement.predicate.repository.commit = "not-an-object".into();
    cases.push(("repository", statement));
    let mut statement = valid.clone();
    statement.predicate.repository.tree_sha256 = "invalid".into();
    cases.push(("repository tree", statement));
    let mut statement = valid.clone();
    statement.predicate.tool.source = "not-an-object".into();
    cases.push(("tool", statement));
    let mut statement = valid.clone();
    statement.predicate.tool.version = " ".into();
    cases.push(("tool version", statement));
    let mut statement = valid.clone();
    statement.predicate.tool.facts_sha256 = "invalid".into();
    cases.push(("tool facts", statement));
    let mut statement = valid.clone();
    statement.predicate.material.inputs_sha256 = "A".repeat(64);
    cases.push(("material", statement));
    let mut statement = valid.clone();
    statement.predicate.material.sha256 = "0".repeat(64);
    cases.push(("material composition", statement));

    for (name, statement) in cases {
        let error = verify_local(&statement, &expected).expect_err(name);
        assert!(!error.to_string().is_empty(), "{name}");
    }

    let mut unsafe_diagnostic = valid.clone();
    unsafe_diagnostic.predicate.outcomes[0].outcome = TestBatchOutcome::Failed;
    unsafe_diagnostic.predicate.outcomes[0].diagnostic = Some("failure\u{1b}[2J".into());
    assert!(verify_local(&unsafe_diagnostic, &expected).is_err());

    let mut oversized_diagnostic = valid.clone();
    oversized_diagnostic.predicate.outcomes[0].outcome = TestBatchOutcome::Failed;
    oversized_diagnostic.predicate.outcomes[0].diagnostic = Some("x".repeat(65 * 1024));
    assert!(verify_local(&oversized_diagnostic, &expected).is_err());
}

#[test]
fn expectation_fields_are_validated_before_they_are_compared() {
    let statement = build(input()).expect("statement");
    let bytes = statement.canonical_json().expect("canonical");
    for field in 0..6 {
        let mut expected = expectation(&statement);
        match field {
            0 => expected.provider_sha256 = "A".repeat(64),
            1 => expected.inputs_sha256 = "short".into(),
            2 => expected.environment_sha256 = "A".repeat(64),
            3 => expected.toolchain_sha256 = "short".into(),
            4 => expected.material_sha256 = "A".repeat(64),
            5 => expected.repository_commit = Some("short".into()),
            _ => unreachable!(),
        }
        let error = parse_and_verify(
            &bytes,
            &expected,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile,
        )
        .expect_err("invalid expected identity");
        assert!(
            error.to_string().contains("digest") || error.to_string().contains("object id"),
            "field {field}: {error}"
        );
    }

    let mut expected = expectation(&statement);
    expected.repository_commit = Some("f".repeat(40));
    assert!(
        parse_and_verify(
            &bytes,
            &expected,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile,
        )
        .expect_err("different valid commit")
        .to_string()
        .contains("changed")
    );
    expected.repository_commit = Some("z".repeat(40));
    assert!(verify_local(&statement, &expected).is_err());
}

#[test]
fn trust_modes_reject_crossed_raw_and_envelope_boundaries() {
    let statement = build(input()).expect("statement");
    let raw = statement.canonical_json().expect("canonical");
    let expected = expectation(&statement);
    let trusted = producer(ProducerPrivilege::Trusted);
    let envelope = dsse(&raw, "AA==");

    let mismatches = [
        (
            TrustPolicy::LocalFile,
            ReceiptTransport::Signed {
                payload_sha256: aplan_attest::sha256_hex(&raw),
                producer: trusted.clone(),
            },
        ),
        (
            TrustPolicy::Signed {
                producer: trusted.clone(),
            },
            ReceiptTransport::LocalFile,
        ),
    ];
    for (policy, transport) in mismatches {
        assert!(parse_and_verify(&raw, &expected, &policy, &transport).is_err());
    }

    assert!(
        parse_and_verify(
            &envelope,
            &expected,
            &TrustPolicy::LocalFile,
            &ReceiptTransport::LocalFile,
        )
        .expect_err("local policy rejects envelope")
        .to_string()
        .contains("raw statement")
    );
    assert!(
        parse_and_verify(
            &envelope,
            &expected,
            &TrustPolicy::SameWorkflow {
                producer: trusted.clone(),
            },
            &ReceiptTransport::SameWorkflow {
                artifact_sha256: aplan_attest::sha256_hex(&envelope),
                producer: trusted.clone(),
            },
        )
        .expect_err("artifact policy rejects envelope")
        .to_string()
        .contains("raw artifact")
    );
    assert!(
        parse_and_verify(
            &raw,
            &expected,
            &TrustPolicy::Signed {
                producer: trusted.clone(),
            },
            &ReceiptTransport::Signed {
                payload_sha256: aplan_attest::sha256_hex(&raw),
                producer: trusted.clone(),
            },
        )
        .expect_err("signed policy needs envelope")
        .to_string()
        .contains("DSSE")
    );
    let error = parse_and_verify(
        &envelope,
        &expected,
        &TrustPolicy::Signed {
            producer: trusted.clone(),
        },
        &ReceiptTransport::Signed {
            payload_sha256: "f".repeat(64),
            producer: trusted,
        },
    )
    .expect_err("signed digest mismatch");
    assert!(matches!(error, ReceiptError::Trust(_)), "{error}");
    assert!(error.to_string().contains("digest"));
}

#[test]
fn authenticated_untrusted_producers_cannot_publish_privileged_evidence() {
    let untrusted = producer(ProducerPrivilege::Untrusted);
    let mut untrusted_input = input();
    untrusted_input.producer = untrusted.clone();
    let untrusted_statement =
        build(untrusted_input).expect("untrusted self-report is representable");
    let untrusted_raw = untrusted_statement.canonical_json().expect("canonical");
    assert!(
        parse_and_verify(
            &untrusted_raw,
            &expectation(&untrusted_statement),
            &TrustPolicy::SameWorkflow {
                producer: untrusted.clone(),
            },
            &ReceiptTransport::SameWorkflow {
                artifact_sha256: aplan_attest::sha256_hex(&untrusted_raw),
                producer: untrusted,
            },
        )
        .expect_err("untrusted identity cannot publish evidence")
        .to_string()
        .contains("untrusted")
    );

    let statement = build(input()).expect("statement");
    let raw = statement.canonical_json().expect("canonical");
    let envelope = dsse(&raw, "AA==");
    let mut replayed = producer(ProducerPrivilege::Trusted);
    replayed.run_id = "different-run".into();
    assert!(
        parse_and_verify(
            &envelope,
            &expectation(&statement),
            &TrustPolicy::Signed {
                producer: producer(ProducerPrivilege::Trusted),
            },
            &ReceiptTransport::Signed {
                payload_sha256: aplan_attest::sha256_hex(&raw),
                producer: replayed,
            },
        )
        .is_err()
    );
}

#[test]
fn malformed_dsse_is_rejected_before_statement_trust() {
    let statement = build(input()).expect("statement");
    let payload = statement.canonical_json().expect("canonical");
    let expected = expectation(&statement);
    let trusted = producer(ProducerPrivilege::Trusted);
    let policy = TrustPolicy::Signed {
        producer: trusted.clone(),
    };
    let transport = ReceiptTransport::Signed {
        payload_sha256: aplan_attest::sha256_hex(&payload),
        producer: trusted,
    };

    let cases = [
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE}),
        serde_json::json!({"payloadType": "wrong", "payload": "AA==", "signatures": [{"sig": "AA=="}]}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "AA==", "signatures": []}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "AA==", "signatures": [{"sig": " "}]}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "AA==", "signatures": [{"sig": "%%%"}]}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "AA==", "signatures": [{"sig": "===="}]}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "%%%", "signatures": [{"sig": "AA=="}]}),
        serde_json::json!({"payloadType": DSSE_PAYLOAD_TYPE, "payload": "", "signatures": [{"sig": "AA=="}]}),
    ];
    for value in cases {
        let bytes = serde_json::to_vec(&value).expect("envelope");
        assert!(parse_and_verify(&bytes, &expected, &policy, &transport).is_err());
    }
    assert!(
        parse_and_verify(b"{\"not\":\"a statement\"}", &expected, &policy, &transport).is_err()
    );
    assert!(parse_and_verify(&[], &expected, &policy, &transport).is_err());
    assert!(
        parse_and_verify(
            &vec![b'x'; MAX_RECEIPT_BYTES + 1],
            &expected,
            &policy,
            &transport
        )
        .is_err()
    );
}

#[test]
fn atomic_publication_and_repository_context_are_real_filesystem_operations() {
    let directory = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    let nested = directory.path().join("nested/receipt.json");
    write_atomic(&nested, b"receipt").expect("atomic write");
    assert_eq!(std::fs::read(&nested).expect("published"), b"receipt");
    assert!(write_atomic(&nested, b"").is_err());
    assert!(write_atomic(&nested, &vec![b'x'; MAX_RECEIPT_BYTES + 1]).is_err());
    assert!(write_atomic(std::path::Path::new("/dev/null/receipt"), b"receipt").is_err());
    assert!(repository_context(directory.path().join("not-a-repository").as_path()).is_err());

    let context = repository_context(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
        .expect("repository context");
    assert_eq!(context.commit.len(), 40);
    assert_eq!(context.tree_sha256.len(), 64);
    assert_eq!(context.status_sha256.len(), 64);
}

#[test]
fn repository_context_refuses_an_unborn_repository() {
    let directory = tempfile::Builder::new()
        .tempdir_in(aplan_core::fixture::fixture_root())
        .expect("fixture");
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(directory.path())
        .status()
        .expect("run git init");
    assert!(status.success(), "git init failed: {status}");

    let error = repository_context(directory.path()).expect_err("unborn HEAD has no tree");
    assert!(matches!(error, ReceiptError::Io(_)), "{error}");
}
