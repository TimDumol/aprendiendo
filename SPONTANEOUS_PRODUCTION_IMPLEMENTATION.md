Implemented spontaneous-production contract (schema 10, recording contract 2)

The default brief now resolves durable policy before filtering inherited drill recommendations. Approved learner preferences are seeded explicitly with `migrate <database> --seed-spontaneous-preferences`; this calls the normal versioned preference service and never overwrites an existing preference row. Opening another database retains mixed practice defaults.

The original mismatch came from inherited grammar recommendations: transformation weight 1.0 and completion weight 0.9 outranked situational response 0.8. `production_focused` was a selection mode, not a guarantee of independent production. Preferences now filter these recommendations before allocation. Contradictions return `no_compatible_activity` and conflicts. No disallowed fallback fills a count.

Preferences and planning

- `get_practice_preferences` returns preferences, version, timestamp and source.
- `update_practice_preferences` accepts a recursively partial typed `patch`, `expected_version`, and a concise learner-intent `source`. Stale writes return `preference_version_conflict`.
- `get_practice_brief` accepts `practice_mode` and `session_overrides: {patch, source}`. Resolution is sourced session override, saved preferences, then defaults. Agent-supplied mode/mix/activity/format parameters must be compatible with this policy.
- To request translation for today, set `practice_mode: controlled`, `drill_mix: translation_only`, and a session override patch setting `default_practice_mode: controlled` and `excluded_drill_types: []`, with the learner's request as source. Saved defaults do not change.
- Spontaneous counts are upper turn bounds within the initial sentence budget. `target_opportunities` are optional and use communicative function labels from taxonomy, never copied grammatical prescriptions. `tutor_context` keeps target patterns and examples separate from `learner_task_constraints`. Activity plans return a response-dependent follow-up strategy.
- `get_learning_context` always includes durable policy, even with `recent_sessions: 0`.

Evidence, validation and reviews

`validate_practice_plan` accepts one or more currently proposed turns, private target opportunities, policy version, session override and preceding context. It returns spans and rule reasons, plus an explicit semantic review requirement. It rejects known construction instructions, supplied llevar/para-que constructions, excluded formats, declared starters/model answers, exact repeated prompts and inconsistent budgets/references. These deliberately limited rules do not prove spontaneity; tutors must check disguised transformations, near-copy answers, unnecessary invention, scenario repetition and whether alternative answers are accepted. Common isolated function words are not a rejection rule.

Recording retains the existing typed items, attempts, observations, activities and reviews. `production_evidence` adds typed attempt/observation assessments, intervention timing, original retry links, exposure, scenario/topic tags, optional target opportunities and variation rationale. Every production observation must be nested under its actual supporting attempt. Every attempt links to an actual delivered item and transcript. Actual prompts are checked again, regardless of their labels. Cueing is separate from post-response hints. Initial sentence counts are explicitly tutor-reported; retries are counted separately. Timing remains nullable and uses the existing provenance checks.

New evidence is stored in `production_sessions` alongside the existing relational records. This avoids rewriting historical rows or inventing missing attempts. `unobserved_targets` excludes explicitly unobserved/ambiguous targets from error counts and onboarding priority; their original observation remains queryable. Communicative outcome is distinct from target accuracy. The existing observation outcome must agree with target accuracy; a fully grammatical alternative uses `target_realization: not_observed`, `target_accuracy: not_assessable`, and `outcome: omitted`.

Raw HTTP `tools/list` already exposed complete models through `$defs`/`$ref`; the weak connector representation was not an untyped Rust model. The server now expands local input references into concrete nested schemas to make these types directly visible to ChatGPT. Protocol tests verify typed collections, nested observation enums, the canonical exercise enum and new evidence definitions survive transport. No generated connector file was edited.

Review decisions have `eligible`, reason codes, supporting/excluded observation numbers, policy version and applied rating. Structural invalidity still rejects the entire transaction. Valid but ineligible proposals save practice and skip schedule updates atomically. Identical requests replay before resolving the current preference version; the stored policy and original decisions survive later preference changes. The added optional request field is omitted when absent, preserving existing request hashes. Old ledger response fields receive defaults on deserialization; no historical reviews are replayed.

The policy is deliberately conservative: all new FSRS reviews require at least two independent observations, two scenario tags, and an explicit tutor rationale establishing meaningful variation. The old brief already requested two cues, but the server previously enforced only one cold retrieval. Controlled, exposed, unknown and assisted evidence is retained but cannot update schedules under contract 2. No second card/track is introduced. Distinct tags and rationale are structural checks, not a linguistic proof of variation.

Every independent failure for the target is considered, even when omitted from the proposal. It must be cited and requires Again; a later assisted success never replaces it. Partial accuracy conservatively counts as failure. Success ratings need a rationale and known effort evidence; Easy additionally retains the existing learner-confirmation and effortless-evidence requirements. One cold observation is still required by the existing rating validator. FSRS parameters and existing schedules are unchanged.

Recent practice and compact context include initial independent/cued/assisted/unknown counts, the full initial-turn denominator, separate retries/transfers, communicative outcomes, target opportunities/observations, policy findings, review decisions, and topic/scenario tags with prompt excerpts. Historical records remain unknown with an unknown initial-turn denominator; no percentage is inferred from session labels. Recent retrieval also reports topic/scenario tag counts across returned initial turns, with unknown-history session counts. Tag/excerpt repetition checks are approximate.

Executable example and tutor sequence

[spontaneous-production-session.json](examples/spontaneous-production-session.json) records two initial communicative turns, an original independent error, an after-round indirect hint and a separate successful retry. [spontaneous-production-result.json](examples/spontaneous-production-result.json) contains the actual fixture result: Again based on the two independent observations, with the assisted retry explicitly excluded. The fixture is exercised end to end by `documented_end_to_end_round_retains_hint_retry_and_failure` in `tests/spontaneous.rs`; it resolves the brief, validates the first prompt, records and retrieves the linked session. These examples belong only in a disposable test database.

Tutor instructions are included in MCP initialization and action descriptions: resolve policy, construct an open prompt, validate and perform semantic review, deliver one turn, adapt after the answer, finish the short round, quote the wrong sentence with an indirect hint, then show original and correction after an unsuccessful retry. Record actual evidence and inspect review decisions. Never extend practice solely to obtain a rating.

Rollout and rollback

1. Take a consistent SQLite online backup and capture history/scheduler counts before release.
2. Run `scripts/release.sh`; startup applies ordered schema migration 10. The image also includes `/usr/local/bin/migrate`.
3. For this explicitly approved learner only, execute `migrate /data/aprendiendo.sqlite3 --seed-spontaneous-preferences` in the running container. Other deployments do not automatically seed.
4. Verify schema, preferences, ordinary production-focused briefs, zero-history context, and unchanged histories/scheduler rows. No synthetic practice is written into production.
5. Before rollback, preserve another complete SQLite backup. Older binaries reject schema 10. Restoring the pre-release database with the previous image is appropriate only if no subsequent learner records would be lost; otherwise implement a forward fix or an explicitly reviewed data-preserving downgrade. Do not blindly restore over newer practice.

Deliberate adaptations: nested preference fields use `after_round: true` and a fixed documented sentence/initial-response budget rather than redundant unit/scope enums; dialogue completion is excluded as an initial format, while open dialogue uses question-answer/situational-response. Source-based activities require an explicit controlled/mixed override. A separate typed evidence envelope preserves existing IDs and request hashes. Backward compatibility was not a design requirement, but historical evidence and idempotent replay were preserved.

The ChatGPT project instructions are managed outside this repository. No local read-only mirror was edited. MCP instructions implement the tutor sequence; any source project-instruction edit requires access to that external source.

## Handover — 2026-09-06

The user requested this handover while the final release was running. The implementation is in the working tree, with production schema 10 and this learner's approved preference version 1 already installed. No commit was created. Do not overwrite the user's original `SPONTANEOUS_PRODUCTION.md` or discard untracked implementation/example files. The user explicitly authorized deployment, SSH and out-of-sandbox work; use `scripts/release.sh` for releases.

### Deployment state at handover

Two releases completed with public health, readiness and OAuth sanity checks:

- `aprendiendo-mcp:release-20260906171352`: main preferences/evidence implementation.
- `aprendiendo-mcp:release-20260906171925`: expanded ChatGPT input schemas and removal of old allocation fields.

The final image is `aprendiendo-mcp:release-20260906172315`. Its release script has passed tests, bootstrap, Docker build and image transfer, and was at the **deploy** step when this handover was written. Completion and final public sanity checks still need confirmation. The running unified exec session is **65654**; poll it with `write_stdin` if still available. Do not start another release merely because the originating turn was interrupted. If the session is unavailable, inspect the running image and service health over SSH.

The final image includes the remaining refinements: opportunities use taxonomy function labels rather than grammar descriptions; generation rules use the saved sentence budget; empty compatible-format sets have an actionable conflict; recent practice reports topic/scenario counts across returned initial turns. All source changes precede this image build.

Production connection details:

- Public service: `https://mars.timdumol.com`.
- SSH: `ssh -i /home/timdumol/.ssh/aprendiendo-deploy timdumol@2.29.6.134` (BatchMode works; passwordless `sudo -n` works).
- Compose file: `/opt/aprendiendo-mcp/app/compose.production.yaml`, service `mcp`.
- Host database: `/opt/aprendiendo-mcp/app/data/aprendiendo.sqlite3`; container path: `/data/aprendiendo.sqlite3`.
- Consistent pre-release backup: `/opt/aprendiendo-mcp/app/data/before-spontaneous-20260906T171015Z.sqlite3`.

The preference seed was already executed successfully through:

```sh
sudo -n docker compose -f /opt/aprendiendo-mcp/app/compose.production.yaml \
  exec -T mcp /usr/local/bin/migrate /data/aprendiendo.sqlite3 \
  --seed-spontaneous-preferences
```

Do not seed again unnecessarily. The operation is idempotent and preserves later edits, but this learner is already on preference version 1, default mode `spontaneous`, provenance `Learner-approved SPONTANEOUS_PRODUCTION.md, 2026-09-06; explicit deployment seed`.

### Verified results

- The complete suite passed outside the sandbox; the latest release script also reports its test step passed. Current suite: **38 tests** (19 library, 3 OIDC, 2 raw HTTP MCP protocol, 14 spontaneous-production integration tests). Initial OIDC failures inside the sandbox were listener permission errors; the authorized out-of-sandbox run passed.
- `cargo fmt --check` and `git diff --check` passed before the final release.
- Raw MCP protocol tests verify 12 tools, concrete nested input types/enums after reference expansion, public preference concurrency conflicts, and durable context with zero recent sessions.
- The live connector reproduction `{count: 3, drill_mix: "production_focused", recent_prompts_per_weakness: 2}` returned only `situational_response` recommendations for both `para_que_subjunctive` and `duration_llevar_gerund`, with saved spontaneous preferences and the 3–5 sentence budget.
- Live `get_learning_context({recent_sessions: 0, weakness_limit: 1})` returned no recent sessions and still included preference version 1.
- Live `get_data_status` reported schema 10, 25 sessions, 37 attempts, 191 observations, 45 scheduler items and 3 existing review events.
- A read-only comparison against the backup verified **every row identical** in `sessions`, `practice_items`, `practice_item_targets`, `attempts`, `observations`, `weaknesses`, `weakness_reviews`, `review_observations`, `scheduler_items`, `scheduler_parameter_sets`, `recorded_requests`, `activity_runs`, `activity_stimuli` and `attempt_reflections`. SQLite integrity and foreign-key checks passed. All 5 historical idempotency ledger rows remain unchanged.
- No synthetic practice was written into the live learner database. Examples and recording tests ran only in disposable local databases.

### Remaining completion checks

1. Confirm release session 65654 finishes successfully and the live image is `release-20260906172315`.
2. Repeat the ordinary production-focused live brief after that release. In particular, verify `target_opportunities[].communicative_function` contains the taxonomy function label rather than a sentence prescribing `llevar`/`para que`, and no target retains an `allocation` field. The earlier live reproduction verified filtering and durable preferences but preceded this final wording refinement.
3. Run the final image's disposable local container smoke test if practical:

   ```sh
   APRENDIENDO_IMAGE=aprendiendo-mcp:release-20260906172315 scripts/smoke_test.sh
   ```

   The script uses local port 8080 and a temporary container/database; ensure that port is available. Its tool count was updated to 12 and it now accepts `APRENDIENDO_IMAGE`. This container test has **not yet been run** during this implementation; Rust HTTP protocol tests have passed.
4. The conversational connector's tool catalog was cached at the original nine tools in this session. Raw published schemas now have expanded input shapes, but adoption/rendering of the new preference and validator actions in a refreshed ChatGPT connector has **not been verified**. Existing read-only connector calls already reached the new server successfully.
5. The original ChatGPT project-instruction source has **not been edited**. No available tool exposing that source was found; do not claim this part is complete or edit a local read-only mirror. MCP initialization/action instructions and the tutor sequence above have been updated. If source access becomes available, apply that same sequence there.
6. Record the final release/smoke/live-check outcomes here, then give the user a concise completion report with the external project-instruction limitation. Keep the backup; do not restore it over subsequent learner practice.

### Where to continue in the code

- `src/production.rs`: typed preferences/patches, policy resolution, prompt rules, explicit evidence and intervention contracts, review decisions, audit/repetition reporting, communicative-function lookup.
- `src/db.rs`: planning filters, atomic recording/replay integration, non-penalizing unobserved-target handling, recent/context output.
- `src/server.rs`: three new MCP tools, public policy errors, concrete input-schema expansion and tutor instructions.
- `src/migrations.rs`: ordered migration 10; `src/bin/migrate.rs` and `Dockerfile`: explicit seed operation and migration binary in the image.
- `tests/spontaneous.rs`, `tests/protocol.rs`, and the two `examples/spontaneous-production-*.json` files: regression coverage and actual disposable-fixture request/result.

Keep the documented limits explicit: rule validation cannot prove spontaneous cognition; taxonomy/scenario tags do not prove semantic variation; transcript text does not establish oral fluency or timing. Under recording contract 2, even controlled reviews now need independent evidence to schedule—this is a deliberate conservative policy change, not an FSRS parameter change. Linguistic correctness, meaningful cue variation and reported effort still depend on supported tutor/learner assessments.

### Completion verification — 2026-09-06

- The old exec session was unavailable. Read-only SSH inspection confirmed the running service uses `aprendiendo-mcp:release-20260906172315`, image ID `sha256:b0f6b87d8cba4accecfcb0dd0f8bd88efcf82184e4b0b799afebca7780f82505`, matching the local image. No additional release was needed.
- Re-ran the release script's public health, readiness, protected-resource metadata, unauthenticated MCP challenge, and OIDC discovery checks independently of deployment: all passed.
- Ran `APRENDIENDO_IMAGE=aprendiendo-mcp:release-20260906172315 scripts/smoke_test.sh`: passed, including all 12 published tools, recording, idempotent replay and conflict handling in a disposable local database. The container was removed by the script; memory use was 11.61 MiB against its 96 MiB limit.
- Repeated the live ordinary brief request `{count: 3, drill_mix: "production_focused", recent_prompts_per_weakness: 2}`. Verified preference version 1, spontaneous policy, 3–5 initial sentences, only situational-response recommendations, optional taxonomy labels `Express purpose` and `Express duration`, and no target `allocation` fields.
- Repeated zero-history learning context: no recent sessions, with durable spontaneous preference version 1 still included.
- Live status reports schema 10, 27 sessions, 39 attempts, 193 observations, 45 scheduler items and 3 review events. Counts have advanced since the handover; this continuation made only read-only live calls and did not write synthetic practice, reseed preferences, or restore the backup. The earlier row-equality audit is a point-in-time result, not a claim that the database has remained unchanged.
- `cargo fmt --check` and `git diff --check` passed. The release's previously passed 38-test suite remains the full-suite result; this continuation additionally verified the actual final container image.
- External follow-ups remain: refreshed ChatGPT connector catalog adoption/rendering is unverified (the current connector still exposes the older catalog), and the ChatGPT project-instruction source is unavailable and has not been edited. Server-side MCP instructions are deployed. No commit was created.
