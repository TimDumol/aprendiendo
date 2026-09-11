# Efficient MCP practice recording: implementation plan

Date: 2026-09-09. Instructions for a smaller coding LLM.

## Outcome and scope

Make a normal tutoring session recordable with one atomic MCP write, without losing original answers, hints, retries, or the evidence needed for conservative FSRS decisions. Make recording performance measurable from structured logs and checked-in post-processing scripts.

Implement the steps below in order. Complete each step's checks before proceeding. This document authorizes a local implementation when assigned to an implementation agent; it does not instruct that agent to deploy or modify production data. If subsequently asked to release, use `scripts/release.sh`.

Do not add a DELE-specific endpoint, automatic fuzzy merging, an internal LLM call, a background recording job, or a separate scheduler. Keep `record_practice_session` available and backward compatible. Add one compact generic tool, `record_tutoring_session`, as the recommended normal tutoring interface. Weakness search/matching is deferred until measurements or actual duplicate records justify it.

Discourage maintenance actions in normal tutoring through initialization and tool descriptions in this implementation. Keep existing actions exposed for compatibility.

Success means:

- A representative DELE session with follow-ups, a new incidental error, a hint, a successful retry, and a stylistic suggestion records in one call.
- A compact request expands deterministically into the existing canonical storage/evidence model.
- Style feedback does not require a weakness or affect error counts or FSRS.
- Existing review rules, transaction rollback, and exact replay behavior remain intact.
- Logs and scripts report action counts, payload sizes, server durations, failures, replays, and explicitly bounded workflow timing.
- The compact example is materially smaller than an equivalent canonical example. Report measured bytes and schema sizes; do not promise an unmeasured latency reduction.

## Step 1 — Read the existing implementation

Read `AGENTS.md`, `Cargo.toml`, `README.md`, `src/server.rs`, the recording types in `src/model.rs`, recording validation/transaction code in `src/db.rs`, and evidence validation/rating decisions in `src/production.rs`. Also read `src/main.rs`, `src/migrations.rs`, `tests/protocol.rs`, `tests/spontaneous.rs`, and `examples/spontaneous-production-session.json`.

Inspect `git status --short` and preserve unrelated changes. Stay out of the mobile recording app and worker: this task concerns MCP tutoring records.

Facts to preserve:

- `record_practice_session` already accepts `new_weaknesses` atomically.
- An incidental new weakness defaults to candidate; a declared target or explicit `active: true` can activate it. Standalone upsert has different defaults.
- `new_weaknesses` currently upserts matching exact keys and can change metadata. Do not use it as an automatic ensure-exists list.
- Every canonical `ObservationInput` requires a weakness key.
- Missing production metadata means unknown evidence; it cannot support a new FSRS review.
- Valid ineligible review proposals save practice and return skip reasons. Structural invalidity rejects the transaction.
- Exact idempotent replay must occur before checking the current preference version.
- The response already includes created keys, record counts, review updates, and review decisions.
- MCP input references are expanded inline to address connector rendering. Preserve typed enum visibility.

## Step 2 — Improve guidance before changing the contract

Update initialization instructions and relevant tool descriptions in `src/server.rs`:

- Reuse the brief's known keys and policy version.
- Record a completed session in one call. Separate turns into items and retries into attempts **within that request**; do not interpret “record each turn separately” as one MCP call per turn.
- Reference known weakness keys directly. Declare only genuinely new weaknesses in `new_weaknesses`; omit explicit activation for incidental discoveries.
- Keep stylistic suggestions and accepted regional alternatives in feedback/notes until structured findings are available.
- Do not perform upserts, taxonomy edits, or confirmation queries as prerequisites to ordinary recording.
- Preserve actual prompts, answers, hints, and retry ordering. Do not infer effort, timing, exposure, or independence.
- Only propose supported reviews. Zero schedule changes is a valid result; never add practice merely to obtain a rating.
- Inspect the recording response. Retry an uncertain write with the identical request and idempotency key.

Make the intended action roles explicit:

| Actions | Guidance |
| --- | --- |
| `get_practice_brief`, `validate_practice_plan`, `record_tutoring_session` once available | Normal tutoring workflow; keep required prompt validation before delivery. |
| `get_learning_context`, `get_recent_practice`, `get_review_queue` | Read when needed; reuse information already returned by the brief or recording response. Do not require all three before recording. |
| `get_practice_preferences`, `update_practice_preferences` | Keep available for preference inspection and learner-requested changes; recording is not a reason to rewrite preferences. |
| `record_practice_session` | Normal recorder until the compact tool is available; then an advanced fallback for unsupported workflows, including activity runs. |
| `upsert_weakness`, `upsert_concept` | Maintenance actions for explicit edits or curation outside routine recording. Never prerequisites for saving a session. |
| `get_taxonomy`, `get_data_status` | Occasional classification or diagnostics; not routine recording prerequisites. |

Use this meaning in the `upsert_weakness` description:

> Maintenance only: explicitly edit an existing weakness or curate learning targets outside session recording. For discoveries during practice, use the recorder's new_weaknesses. Do not call this before recording a session.

Also state that standalone upsert can activate a new weakness and overwrite existing metadata. Give `upsert_concept` an analogous maintenance description directing tutors to defer taxonomy curation during recording. These are workflow instructions, not a new permission mechanism: do not add confirmation calls or prompts for maintenance the learner has already requested.

Descriptions guide tool selection but do not reduce the published schema catalog. Retain all current actions in this implementation and report total catalog size honestly.

Add a concise canonical one-call example to `examples/` and link it from `README.md`. Include a new incidental weakness, an existing target, a separate retry, and style feedback. Use a disposable seeded database in its integration test; no production calls.

Check: the example records once, creates the incidental weakness as candidate, keeps the retry distinct, and does not need an upsert or post-record read. Test reads may verify persistence.

Check `tools/list` in the protocol tests: maintenance descriptions and the canonical recorder's current role must survive transport. Existing maintenance tools remain callable; adding the compact recorder is the only tool-count change. Do not claim description assertions alone prove that an LLM will avoid unnecessary calls; use the action logs to evaluate actual behavior later.

## Step 3 — Structured performance logs

Suggested files: `src/telemetry.rs`, `src/main.rs`, `src/server.rs`, and targeted recording instrumentation in `src/db.rs`. Export the module from `src/lib.rs`.

### Event format and coverage

Add JSON logging through `tracing-subscriber`'s JSON feature. Support `LOG_FORMAT=json|text`, with JSON as the documented production setting and startup validation for unsupported values. Ensure the existing deployment environment actually selects JSON when this change is released. Preserve normal diagnostics; analytics must filter on a stable `event` field.

Emit versioned `mcp_tool_started` and `mcp_tool_completed` events for every MCP tool invocation, plus a separate structured `mcp_validation_failed` diagnostic for actionable validation results. Instrument the dispatcher, not only typed method bodies, so malformed arguments and unknown tools are represented. Inspect the installed rmcp source to find the supported dispatch hook; preserve macro routing and authorization behavior. Do not count a compact tool's internal service call as a second MCP invocation.

Each event has these allowlisted fields where applicable:

| Field | Meaning |
| --- | --- |
| `event_version` | Initially 1 |
| `event` | Stable event name |
| `timestamp` | UTC timestamp from the JSON formatter |
| `process_instance_id` | Random opaque ID for each server start |
| `call_id` | Server-generated random opaque invocation ID, shared by start/completion |
| `tool` | Known tool name; use `unknown` for unrecognized user-supplied names |
| `arguments_json_bytes` | UTF-8 byte count of serialized parsed arguments; not raw HTTP bytes or tokens |
| `duration_ms` | Completion only; monotonic elapsed dispatch time including validation and store wait |
| `outcome` | `success`, `invalid_argument`, `unknown_reference`, `idempotency_conflict`, `internal_error`, or another documented bounded transport error class |
| `record_status` | `created` or `replayed`, for successful recording |
| `session_id` | Successful recording's database session ID |
| `item_count`, `attempt_count`, `observation_count`, `new_weakness_count`, `review_applied_count`, `review_skipped_count` | Numeric result counts where available |

Capture MCP error results even if HTTP status is 200. Prefer one completion emitter around dispatch to avoid duplicate events. An unmatched start is an incomplete/cancelled/crashed call, not a successful or zero-duration call. Do not catch panics simply to synthesize success or change cancellation behavior.

Add recording-only phase durations where feasible: deterministic expansion for the compact tool, waiting for the store/connection, and transaction execution. Inspect actual store concurrency before naming phases; do not call combined queue-plus-execution time “database execution.” Pass correlation explicitly or propagate the tracing span into blocking work. Phase values are components of action duration, not additional latency to sum again.

Action and aggregate events must not log argument bodies, transcripts, prompts,
feedback, weakness descriptions/keys, raw idempotency keys, authorization
headers, cookies, or arbitrary client metadata. The single-learner deployment
may additionally emit an explicitly separate, allowlisted `mcp_tool_text`
event for recording text; routine analyzers must filter that event by default
and an investigation can opt in. Error classification must not copy validation
messages containing learner text. Existing general diagnostic logs are not the
analytics contract; avoid broad logging rewrites in this task.

### What timing can establish

The server can measure invocation duration and observe gaps between invocations. It cannot measure LLM reasoning or network round trips directly, and cannot see time before the first request arrives.

Do not group records into sessions by adjacency alone: preparation, tutoring, recording, concurrent clients, and unrelated maintenance can interleave. `session_id` identifies a successful recording but cannot retroactively label preceding upserts.

For an observed slow workflow, the analysis script must accept an explicit set of call IDs or a user-selected UTC window. A time window is a window report, not proof every included call belongs to one session. Compute:

- Window from first included start to last included completion.
- Sum of action durations (work time; may exceed elapsed time under concurrency).
- Union of action intervals within the window (server-busy elapsed time).
- Window minus that union (unattributed gap time, potentially model, network, client, or user delay).

Optional client start/end timestamps supplied to the script may establish user-observed total duration, clearly labeled as external input. Do not add a new MCP call merely to start a timer, and do not ask the tutor to invent precise timestamps.

### Post-processing scripts

Implement `scripts/analyze_mcp_logs.py` using Python's standard library. This is an offline analysis dependency only. It reads JSONL files or stdin and emits a readable report by default, plus `--format json|csv` for automation. Support `--from`, `--to`, `--tool`, and repeated `--call-id` filters. Document timestamp and CSV semantics.

Report per-tool calls, successes/failures, record creations/replays, p50/p95/max duration, argument-byte distribution, counts of applied/skipped reviews, incomplete calls, and the timing breakdown above. Define the percentile method and units. Deduplicate repeated events by process/call/event identity; report conflicting duplicates. Handle interleaved events, unrelated application logs, empty input, and malformed lines with explicit skipped-line counts. Do not silently turn missing fields into zero.

Implement `scripts/compare_recording_payloads.py`, also standard-library-only. It compares canonical and compact JSON fixtures by serialized UTF-8 bytes and reports absolute/percentage savings. Accept a saved `tools/list` response to report each recorder's published input-schema bytes and total catalog bytes. Label bytes as bytes, not estimated tokens. Adding a compact action can enlarge the full catalog even while reducing the selected tool schema and generated request.

Add `docs/mcp-recording-performance.md` with field definitions, collection commands, script usage, interpretation limits, and a before/after worksheet. Example commands once implemented:

```sh
docker compose -f compose.production.yaml logs --no-color --no-log-prefix mcp > /tmp/mcp-recording.jsonl
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl --format json
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl --call-id CALL_ID
python3 scripts/compare_recording_payloads.py examples/tutoring-canonical-session.json examples/tutoring-compact-session.json
```

These are documentation examples, not instructions to access production now. Explain how to export unprefixed JSON logs; don't build a heuristic parser for arbitrary Docker prefixes.

Checks: capture actual dispatcher events in tests for success, validation failure, unknown tool, recording replay, and recording conflict. Use recognizable secret/transcript sentinels and assert they are absent from action/aggregate telemetry, while the allowlisted transcript sentinel is present only in the separate `mcp_tool_text` event. Test the analysis scripts with synthetic JSONL fixtures covering overlap, missing completions, duplicates, malformed lines, filtering, text opt-in, validation diagnostics, and external timing bounds. Confirm logging does not change tool results or transaction behavior.

## Step 4 — Add structured findings without forcing weaknesses

Add an optional `findings` collection to the canonical request and persist it atomically with the session in a small new table. Follow the next available migration number; do not edit an already applied migration. Suggested fields:

- Optional `practice_item_no` and `attempt_no` for linkage. If both supplied, they must refer to the same turn.
- `assessment_kind`: `error`, `awkward`, `regional_variant`, `stylistic_improvement`, `accepted`.
- `original`, optional `suggestion`, and optional concise `note`.

Findings are feedback records, not weakness observations. For a tracked error, attach a real observation as well; do not count both as separate errors. A finding alone never creates a weakness or updates a schedule. `awkward` is not an automatic promotion rule; recurrent patterns can later become deliberately chosen targets.

Bound collection and text sizes; validate references before commit. Return findings in the relevant recent-practice detail response so structured data is recoverable. Add defaulted summary fields for finding counts by kind to recording results, preserving old ledger response deserialization. Keep existing observation counts explicitly described as observations, not unique errors.

Preserve historical idempotency hashes: omit the new field when empty during canonical serialization. Do not clear the ledger. Test replay of an old payload/stored response, including after a preference update.

Check: a style-only session needs no weakness; invalid finding references roll back all writes; findings survive retrieval and replay.

## Step 5 — Implement one compact recorder

Suggested file: `src/tutoring.rs`. Use typed structs with `deny_unknown_fields`, bounded collections, and existing enums where meanings match. Add the MCP action and protocol assertions; update the expected tool count.

Use this nesting, with arrays establishing stable order:

```text
session: idempotency_key, exercise_type_key, optional dates/topic/notes
         optional policy_version/session_overrides, new_weaknesses
         turns[]
           drill_type, prompt, optional target_weakness_keys
           attempts[]
             transcript, response_mode
             optional evidence: kind, cueing, exposure, provenance,
                                communicative outcome, scenario/topic,
                                explicitly reported sentence count/effort
             observations[]: weakness_key, outcome, optional assessment facts
             findings[]: assessment_kind, original, suggestion, note
             interventions_after[]: kind, actual text, target_weakness_keys
         optional reviews[]: target, rating/evidence and local observation references
```

Use 1-based `{turn, attempt, observation}` references only where cross-links are necessary, such as review evidence and original-attempt links. Array position supplies all ordinary IDs; the caller should not number each object. Derive canonical item/attempt/observation/intervention numbers deterministically in traversal order. Never depend on wall-clock time, randomness, or current database matches during expansion.

Store each transcript once in the expanded canonical attempts, rather than copying it into item response as well. Original learner text and tutor corrections must remain distinct. Map findings to the table from Step 4. Do not add transcript parsing or automated linguistic assessment.

Preserve explicit intervention ordering, including a final correction with no subsequent retry. For retries across turns, require an explicit original-attempt reference; do not assume all noninitial attempts are the same kind. Reject impossible ordering and out-of-range references with paths the caller can fix.

The compact schema must retain the information needed by current evidence validation: policy version, actual prompt links, prompt cueing versus later hints, prior exposure, provenance, scenario/topic variation, and rating support. Mechanical mappings may derive duplicate enums or IDs when logically equivalent. Missing semantic evidence remains unknown. Do not default missing cueing to `none_detected` or missing exposure to `none_known`.

For record-only sessions, support omission of the production-evidence envelope just as the canonical API does. Retain raw attempts, observations, findings, and actual intervention text in durable feedback if no production envelope is supplied; document this mapping. If partial evidence is supplied, normalize missing semantic values conservatively or reject incomplete structural fields with a precise error. Do not invent provenance narratives or scenario tags. Reviews still pass through the existing eligibility rules.

Keep explicit review proposals; don't automatically rate every error or every successful correction. Preserve the supported route for a real eligible review, including variation rationale and effort evidence. An initial error followed by a prompted success alone must not schedule a review.

Expand to `RecordPracticeSessionRequest` and call the shared store recording method directly. Do not call other MCP actions, upsert independently, or duplicate transaction/scheduler logic. Exact compact retries must expand identically and replay the same canonical ledger entry. Preserve the original response even if preferences have changed. Return the canonical response with the new finding summaries; no second read.

Keep advanced activity runs on the canonical endpoint initially. State that boundary in the new tool description. Both tools still support the canonical exercise-type enum, including DELE A2; do not require a DELE-specific implementation branch.

After adding the tool, update guidance to prefer it for supported tutoring sessions. Do not remove schema inlining unless a separate compatibility test demonstrates that the connector can handle the alternative.

## Step 6 — Behavioral verification and measurements

Add equivalent canonical/compact fixtures covering the representative one-call tutoring session. Verify persisted semantics, not only serialized expansion snapshots:

1. Existing weakness reused; new incidental weakness created as candidate; no preparatory upserts.
2. Original incorrect response, actual indirect hint, and correct prompted retry preserved separately; no unsupported review applied.
3. Two genuinely independent varied observations with required metadata can still produce the same supported review result as the canonical fixture.
4. Stylistic and accepted findings create no weakness and do not contribute error observations.
5. Missing assistance information cannot schedule a review.
6. Invalid references or structural inconsistency roll back the entire session, findings, and new weaknesses.
7. Identical retries replay once, changed payload conflicts, and replay survives a preference change.
8. Typed schema fields/enums survive `tools/list`; unknown fields fail visibly.

Run `cargo fmt --check`, `cargo test`, and the new Python script tests. Avoid synthetic production writes. Use local disposable databases for a small repeatable measurement of server processing; distinguish created requests from replays and report the sample size. There is no latency threshold tied to CI hardware.

Run the payload/schema comparison script and record results in the performance document. A reasonable target is at least 30% fewer request bytes for the representative full-evidence fixture, achieved by removing duplication and manual links rather than omitting evidence. If missed, explain the measured reason and simplify the shape where safe. Do not remove evidence merely to meet that number.

## Step 7 — Handoff

Update `README.md` with the preferred one-call workflow, action roles from Step 2, and links to both recorder examples and the performance guide. Document JSON logging configuration and runnable post-processing commands. Clearly distinguish measured server improvements from unmeasured end-to-end model performance.

Report changed files, completed checks, fixture/schema byte comparisons, any measured local timings, and outstanding connector adoption checks. Do not claim the live connector has refreshed or that five minutes has become a particular number of seconds without an actual observed run.

Stop after the local implementation and verification unless release was separately requested. Automatic fuzzy matching, and a DELE-specific endpoint remain deferred.
