# MCP recording performance guide

The server emits a small analytics contract through `tracing`. Set
`LOG_FORMAT=json` in production so each log line is an unprefixed JSON object;
`LOG_FORMAT=text` remains available for local debugging. The action and phase
events contain identifiers, counts, classifications, byte counts, and
durations. They do not contain request bodies, transcripts, prompts, feedback,
weakness keys, authorization data, or raw idempotency keys.

Recording calls also emit a separate `mcp_tool_text` event containing an
allowlisted array of learner-facing text fields (prompt, transcript, produced
text, corrections, findings, and related bounded text). This event is
deliberately separate so routine analysis can discard it without parsing or
loading learner text. It never includes authentication headers, cookies,
idempotency keys, task references, or arbitrary client metadata. The server
limits each captured value to 4,000 UTF-8 bytes and the event to the bounded
recording request size.

The stable event names are `mcp_tool_started`, `mcp_tool_completed`,
`mcp_recording_phase`, `mcp_validation_failed`, and `mcp_tool_text`. Every
event has `event_version: 1`, a UTC `timestamp`,
`process_instance_id`, `call_id`, and `tool`. The start event may include
`arguments_json_bytes`, the UTF-8 size of the parsed arguments object. The
completion event includes monotonic server `duration_ms`, a bounded `outcome`,
and recording result fields when available: `record_status`, `session_id`,
`item_count`, `attempt_count`, `observation_count`, `new_weakness_count`,
`review_applied_count`, and `review_skipped_count`. A missing optional field is
missing data; it is not a zero.

Validation failures additionally emit `mcp_validation_failed` with a bounded
`code`, `path`, `expected`, and `correction`. The diagnostic excludes the
validator's original prose so a future validator cannot accidentally copy a
submitted transcript into aggregate logs. Use `--include-text` only when the
separate learner-text payload is needed for an investigation:

```sh
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl --format json --include-text
```

Without that flag (the default), the analyzer skips text events and reports
how many were skipped. To create a compact action-only file before opening it
in a context window, use:

```sh
jq 'select(.event != "mcp_tool_text")' /tmp/mcp-recording.jsonl > /tmp/mcp-actions.jsonl
```

Recording phase events use the same correlation IDs and have `phase` and
`duration_ms`. Current phase names are `compact_expansion`,
`store_connection_wait`, and `transaction_execution`. The first is pure
request traversal. The second measures waiting for the process's SQLite
connection mutex. The third covers the transaction scope. These phases are
components of the completion duration and must not be added to it again.

To collect a local or container log, preserve the JSON lines without Docker's
timestamp or prefix. For example:

```sh
docker compose -f compose.production.yaml logs --no-color --no-log-prefix mcp > /tmp/mcp-recording.jsonl
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl --format json
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl --call-id CALL_ID
python3 scripts/compare_recording_payloads.py examples/tutoring-canonical-session.json examples/tutoring-compact-session.json
```

The analyzer also accepts standard input, `--from` and `--to` UTC RFC 3339
timestamps, `--tool`, repeated `--call-id`, and optional external bounds:

```sh
python3 scripts/analyze_mcp_logs.py /tmp/mcp-recording.jsonl \
  --from 2026-09-09T10:00:00Z --to 2026-09-09T10:10:00Z \
  --client-start 2026-09-09T10:00:03Z --client-end 2026-09-09T10:05:20Z \
  --format csv
```

The analyzer deduplicates repeated events by process, call, event, and phase.
It reports identical duplicates and conflicting duplicates separately. It also
reports malformed JSON, unrelated lines, calls with a start but no completion,
and completions with no start. Its p50 and p95 values use linear interpolation
at ranks `p * (n - 1)` and are expressed in milliseconds, except argument
sizes, which are bytes. CSV rows use `scope`, `tool`, `metric`, `value`,
`unit`, `observed`, and `missing`; missing values are blank rather than zero.

Timing is intentionally bounded. For selected calls, the report shows the
first included start and last included completion, the sum of completed action
durations, the union of paired start/completion intervals, and the elapsed
window minus that union. The action sum can exceed elapsed time when calls
overlap. The remaining gap can include model generation, network transfer,
client work, user delay, or calls outside the selected IDs. A selected UTC
window is an observation window and does not prove that adjacent calls belong
to one tutoring session. Supplying `--client-start` and `--client-end` adds an
explicitly external user-observed total; it does not turn that value into a
server measurement.

`compare_recording_payloads.py` parses both fixtures and serializes them with
sorted keys, compact separators, and UTF-8 encoding. Its values are bytes, not
token estimates. A saved `tools/list` response can be supplied with
`--tools-list` to report the published input schema size for each recorder and
the complete catalog size:

```sh
python3 scripts/compare_recording_payloads.py \
  examples/tutoring-canonical-session.json \
  examples/tutoring-compact-session.json \
  --tools-list /tmp/tools-list-response.json --format json
```

The compact request is deliberately complete: it retains the evidence needed
for conservative review decisions, so its request reduction is smaller than
the schema reduction. The representative request saves 15.71%, below the 30%
planning target, because the canonical fixture contains little duplicated
evidence once its explicit links are normalized. Removing evidence to reach the
target would undermine review validation, so the implementation keeps the
measured shape. The compact schema removes caller-assigned item,
attempt, observation, intervention, and production-evidence numbering while
keeping the actual evidence. It expands deterministically to the existing
canonical request.

The checked-in representative measurement on 2026-09-09 used the two tutoring
fixtures and normalized UTF-8 JSON:

| Measurement | Canonical | Compact | Difference |
| --- | ---: | ---: | ---: |
| Serialized request | 5,492 bytes | 4,629 bytes | 863 bytes / 15.71% |
| Raw pretty fixture | 7,453 bytes | 7,038 bytes | 415 bytes / 5.57% |
| Published input schema | 36,897 bytes | 24,473 bytes | 12,424 bytes / 33.67% |

The same captured `tools/list` catalog contained 13 tools and 103,403
normalized catalog bytes. Adding a compact action can increase the complete
catalog even while reducing the selected recorder schema. The request sample
does not claim a particular end-to-end latency improvement: it preserves full
production evidence, and model and network time are outside the server timer.

For a repeatable local processing sample, the disposable-database integration
test records five created compact requests and five exact replays, and prints
the two sample sizes and mean local recording durations when run with
`--nocapture`:

```sh
cargo test --offline --test tutoring local_recording_measurement_reports_created_and_replayed_samples -- --nocapture
```

Record those values in a worksheet with the Rust version, machine, fixture
revision, and sample counts. Created and replayed requests exercise different
database paths, so combine them only when the question calls for an overall
average.

| Run date | Fixture/revision | Created samples | Created p50/p95 ms | Replay samples | Replay p50/p95 ms | Notes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| 2026-09-09 | tutoring fixtures in this revision | 5 | 5.141 / 5.886 | 5 | 0.331 / 0.418 | disposable SQLite database; local test process; mean 5.319 / 0.349 ms |

This worksheet is a place for observed measurements. It should not be filled
with inferred model or network timing, and it should not be used to claim that
a normal tutoring exchange takes a fixed number of seconds.

The retained production sample from 2026-09-09 contained two rejected
`record_practice_session` calls before session 37 was eventually created. Their
argument sizes were 21,329 and 16,418 bytes, and both server dispatches were
under one millisecond. The old deployment logged only the outcome and size,
not the error body or separate text event, so their exact invalid fields cannot
be recovered. New calls expose the structured validation diagnostic and keep
learner text available in the separately filterable event when a future
investigation needs it.
