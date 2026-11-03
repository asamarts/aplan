# ADR-0056: Portable test evidence is a provider statement, not a plan verdict

Date: 2026-10-24
Status: Accepted
Tasks: `A1-644`

## Context

Component jobs already execute tests that a later plan-verification job needs as L3 evidence.
Repeating those tests merely to move a citation from resolved to executed is expensive, but a
pass/fail file is not evidence: it says neither exactly what ran nor which runner, inputs, tools and
producer made the answer current. A build cache solves a different problem. It may return reusable
outputs after a key hit; an evidence receipt must let a consumer reject an incomplete, stale or
unauthenticated claim without trusting the producer's own trust label.

The local observation cache from ADR-0053 cannot simply be copied between jobs. It is disposable,
private host state, records a catalogue rather than execution outcomes, and deliberately binds its
checkout location. The batch result from ADR-0054 is attributable and complete, but its nonce-bound
temporary file is valid for only one local invocation and authenticates no CI producer.

## Decision

**1. One receipt describes one provider observation.** `aplan test receipt <runner>` exports an
in-toto Statement v1 whose predicate type is
`https://aplan.dev/attestation/test-evidence/v1`. The predicate uses the batch protocol's typed
outcomes and contains one exact, sorted result for every requested identity. It contains no task,
plan digest or plan verdict. A consumer recomputes ambiguity, relevance, policy and the whole-plan
verdict against the exact plan it is judging. Plan-only edits therefore invalidate that derived
verdict without invalidating unchanged provider work.

The in-toto subject is derived from the complete typed predicate. The committed JSON Schema is
derived from the Rust statement type, unknown fields and schema majors are refused, and the raw
statement has one canonical key-ordered JSON encoding. Portable transport is a file or stream; no
hosted service is part of the format.

**2. Currency is a composition of exact identities, not a timestamp.** The predicate records:

- the runner's one-based position, stable label and complete serialized definition, including its
  command or batch adapter;
- the complete listed namespace, exact requested identities and typed outcomes;
- SHA-256 identities for the exact `evidence.observation_inputs` declaration, every file it
  matches and each executable mode,
  every inherited environment key/value, the platform, configured executable/interpreter bytes,
  existing file arguments (including scripts outside the repository), and applicable Rustup state;
- the aplan version, source, capability-document digest and running binary digest;
- the Git commit, complete tree identity and exact dirty-status identity as audit context; and
- start/completion timestamps as audit facts only.

The composed material digest is re-derived from its component digests on import. Receipt creation
observes material and repository context both before and after execution and publishes nothing if
either moves. Revision movement may be permitted only when every stronger provider, material,
environment, toolchain and aplan identity remains exact. Wall-clock age is never proof of currency.

**3. Complete and successful are separate.** `complete` means every exact request has one
attributable terminal row; it does not mean every row passed. In addition to unconditional
`passed` and ordinary `failed`, the shared protocol distinguishes `skipped`, `expected_failure`,
`unexpected_pass`, `retried_passed`, `flaky`, `cancelled`, `timed_out`, `resource_breach`,
`framework_crash`, `incomplete`, and the original `stopped` and `unavailable` spellings. Only an
unconditional single-attempt pass can become L3 proof. A plain `passed` row carrying more than one
attempt is malformed rather than a retry-created green.

A provider-wide crash, failed skip query, missing result, partial result or selector absent from
the complete catalogue prevents publication. An explicitly attributable non-passing row may be
complete while still unable to satisfy evidence.

**4. Integrity and producer authentication are independent.** The same statement supports three
consumer-selected policies:

- a local file trusted because the caller controls its filesystem boundary;
- an unsigned same-workflow artifact whose independently obtained digest and authenticated
  repository/workflow/run/event/privilege coordinates match; or
- a DSSE envelope whose signature and producer identity were authenticated by the hosting or
  signing system and whose exact payload digest matches.

The JSON's producer fields are self-reported until compared with those external facts. An artifact
digest proves bytes, not who produced them. A pull-request or other restricted producer is marked
`untrusted` and cannot satisfy a policy requiring privileged evidence. Aplan parses DSSE and binds
the externally authenticated payload; it does not invent a certificate authority or treat a
non-empty signature field as cryptographic verification.

**5. Publication and diagnostics fail closed.** The canonical document is size-bounded. Adapter
diagnostics are byte-bounded, line/width-bounded, terminal-safe and passed through the existing
credential redactor. A file is written to a private sibling, flushed and atomically renamed only
after the complete statement exists; a crash before rename leaves the previous destination whole.
Import rejects truncation, unknown versions and fields, noncanonical payloads, forged subjects,
duplicate identities, contradictory retry metadata, unsafe diagnostic controls, changed identity,
transport/policy mismatch and a valid authenticated envelope carrying the wrong predicate.

## Consequences

- A component job can publish what it already ran without coupling the format to GitHub, GitLab or
  an aplan service. Artifact retention and access policy remain the repository operator's choice.
- Receipts are correctness-bearing inputs and must not be placed in an ordinary correctness-neutral
  compiler cache. Losing one costs a rerun; accepting the wrong one can create a false proof.
- Exact inherited-environment identity is conservative. Cross-job reuse requires jobs whose
  relevant execution environment is genuinely identical; a later policy may introduce an explicit
  environment contract only if it can preserve fail-closed treatment of undeclared variables.
- This decision establishes export and the hostile import/trust boundary. Whole-plan zero-provider
  consumption, changed-input incremental execution and adopter rollout are separate tasks
  (`A1-645` through `A1-647`).

## Alternatives

| option | why not |
|---|---|
| Reuse JUnit, TAP or a framework JSON report directly | they do not bind the exact request, complete catalogue, provider definition, material or authenticated producer |
| Store the plan verdict in each component artifact | component jobs do not own the exact final plan, and plan-only edits would invalidate reusable provider work |
| Treat artifact SHA-256 as authentication | a digest detects changed bytes but says nothing about repository, workflow, event privilege or who uploaded them |
| Require DSSE for every adopter | local and same-workflow artifacts have useful trust boundaries where signing is unavailable; making signing mandatory would replace a portable baseline with a hosting dependency |
| Accept receipts by age or source commit alone | neither detects changed generated inputs, environment, adapter, external tool binary or retry semantics |
| Extend the disposable local observation-cache envelope | it has a host-local location identity and catalogue-only payload by design; widening it would merge a performance cache with a correctness artifact |

## Primary references

- [in-toto Statement v1](https://github.com/in-toto/attestation/blob/main/spec/v1/statement.md)
- [Dead Simple Signing Envelope](https://github.com/secure-systems-lab/dsse/blob/master/envelope.md)
- [Bazel Remote Execution action and command identities](https://github.com/bazelbuild/remote-apis/blob/main/build/bazel/remote/execution/v2/remote_execution.proto)
- [GitHub workflow artifacts](https://docs.github.com/en/actions/concepts/workflows-and-actions/workflow-artifacts)
- [GitHub artifact attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
- [GitLab job artifacts](https://docs.gitlab.com/ci/jobs/job_artifacts/)
