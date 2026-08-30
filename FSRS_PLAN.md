# FSRS and Practice Planning Implementation Plan

## 1. Purpose

The taxonomy, evidence, historical-bootstrap, and migration rules in
`TAXONOMY_PLAN.md` extend and control this plan where they overlap. In
particular, legacy observations remain historical evidence and must not be
reconstructed into inferred FSRS reviews.

This document specifies how to replace Aprendiendo's current home-grown spaced
repetition scheduler with FSRS-6 and how to extend the MCP so it can support
mixed Spanish drills without trying to generate language itself.

It is written to be implementable by an agent that has no context beyond this
repository and this file. Treat the decisions marked **Decision** as settled
unless implementation uncovers a technical impossibility.

The two primary workflows are:

1. Generate a batch of six exercises targeted at current weaknesses, conduct
the exercises in ChatGPT, correct each answer, and record the evidence.
2. Conduct a 4/3/2 fluency exercise using an external timer, compare the three
transcripts, record persistent/resolved weaknesses, and update only the
deliberately reviewed weaknesses in FSRS.

DELE-specific planning is explicitly deferred.

## 2. Goals

* Use the official FSRS Rust implementation instead of custom interval/ease
formulas.
* Keep an append-only review history sufficient to reproduce and later optimize
FSRS state.
* Schedule weaknesses (knowledge components), not one-off generated prompts.
* Store each generated prompt and learner response separately.
* Let the MCP select targets and return recent prompts to avoid; let ChatGPT
generate the actual exercise language.
* Support translation and other drill types through one practice-brief tool.
* Separate raw observations from the single FSRS rating applied to a weakness
for a completed session.
* Make all recording, weakness creation, item storage, observation storage, and
FSRS state changes atomic and idempotent.
* Preserve existing sessions, attempts, observations, and weaknesses.

## 3. Non-goals

* The Rust server will not contain an LLM, embedding model, or natural-language
exercise generator.
* The server will not claim to detect semantic novelty. It will provide recent
prompts and reject exact duplicates only when requested.
* The server will not infer audio properties from transcripts. In particular,
it must not infer actual speaking duration, words per minute, pause count,
filler count, or acoustic fluency.
* FSRS will not schedule the general habit of doing 4/3/2. FSRS schedules
retention of targeted weaknesses. A recurring fluency-practice cadence would
be a separate future feature.
* Personalized FSRS parameter optimization is not part of the first release.
The schema must retain enough review history to add it later.
* DELE A2 task modeling is deferred.

## 4. Current implementation and problems

The current MCP exposes six tools in `src/server.rs`:

* `get_learning_context`
* `get_recent_practice`
* `record_practice_session`
* `get_review_queue`
* `upsert_weakness`
* `get_data_status`

The current weakness scheduler is implemented by `update_review_schedules` in
`src/db.rs`. It stores `due_date`, `interval_days`, `ease_factor`,
`repetitions`, `lapses`, and `last_reviewed` on each weakness. It applies the
worst observation seen anywhere in a session and uses fixed interval rules.

This has four important problems:

1. A single incorrect use forces a lapse even if several later uses are
correct.
2. An error in the first 4/3/2 round overrides successful correction in later
rounds.
3. Incidental errors are scheduled as if they were deliberate retrieval tests.
4. Six exercises are currently stored in one transcript, so prompts and answers
cannot be retrieved or evaluated individually.

The implementation must correct these problems without deleting legacy data.

## 5. Core domain decisions

### 5.1 FSRS memory unit

**Decision:** One active weakness is one FSRS memory unit in the first version.

Examples are `clitic-order`, `por-vs-para`, or
`preterite-vs-imperfect`. Generated prompts are retrieval cues for the weakness;
they are not independent FSRS cards.

Every review event must record a `retrieval_mode` so a future version can split
a weakness into tracks such as `clitic-order:controlled_production` and
`clitic-order:spontaneous_production`. Do not create those tracks now because
the existing dataset is too sparse.

### 5.2 One review per weakness per session

**Decision:** A session may produce at most one FSRS review event for a given
weakness.

The event is based on all relevant item/attempt evidence from that session. Raw
observations remain separate and may be numerous.

The three rounds of a 4/3/2 exercise are one practice event, not three spaced
reviews. They happen too close together to represent three independent tests of
long-term retention.

### 5.3 Targeted versus incidental evidence

**Decision:** Only a deliberately targeted weakness may update FSRS by default.

Incidental observations are still stored and may create or reactivate a
weakness, but they do not change that weakness's FSRS state unless the request
explicitly includes an FSRS review for it.

### 5.4 Who supplies the rating

**Decision:** `record_practice_session` accepts one explicit FSRS rating per
reviewed weakness. The server validates and applies the rating but does not infer
it from prose.

ChatGPT should suggest the rating from the evidence and expose its reasoning to
the learner. The learner may override it before recording. This is necessary
because a transcript cannot reveal mental effort reliably, especially for the
`easy` rating.

### 5.5 Rating meanings

Use the FSRS rating values exactly:

| Value | Name | Operational meaning for this project |
| --- | --- | --- |
| 1 | `again` | Target could not be retrieved, was omitted, or the required construction was wrong. |
| 2 | `hard` | Retrieval was inconsistent, prompted, partially correct, or corrected only in a later immediate attempt. |
| 3 | `good` | Target was produced independently and acceptably. |
| 4 | `easy` | Target was independently produced across varied cues and the learner explicitly judged recall effortless. Use sparingly. |

ChatGPT must not infer `easy` merely from a clean transcript. Unless the learner
explicitly supplies that judgment, the highest automatic suggestion is `good`.

Suggested aggregation for two controlled-production items targeting the same
weakness:

| Evidence | Suggested rating |
| --- | --- |
| Both incorrect/omitted | `again` |
| One correct and one incorrect, or prompted correction | `hard` |
| Both independently correct | `good` |
| Both correct and learner explicitly reports effortless recall | `easy` |

Suggested aggregation for a targeted weakness in 4/3/2:

| Evidence across rounds | Suggested rating |
| --- | --- |
| Persistently wrong or absent | `again` |
| Wrong initially but resolved in a later round | `hard` |
| Correct independently throughout | `good` |
| Correct throughout and explicitly judged effortless | `easy` |

These mappings are interaction guidance, not server-side hidden heuristics.

## 6. FSRS integration

### 6.1 Library

Use the official Open Spaced Repetition Rust crate named `fsrs`:

* Repository: [https://github.com/open-spaced-repetition/fsrs-rs](https://github.com/open-spaced-repetition/fsrs-rs)
* Algorithm: [https://github.com/open-spaced-repetition/awesome-fsrs/wiki/The-Algorithm](https://github.com/open-spaced-repetition/awesome-fsrs/wiki/The-Algorithm)

Pin a concrete compatible crate version in `Cargo.toml` and commit the resulting
`Cargo.lock`. Do not reimplement FSRS formulas in SQL or Rust.

The relevant scheduling API conceptually takes:

* an optional previous `MemoryState { stability, difficulty }`;
* desired retention;
* elapsed whole days since the previous review;
* one of the four ratings;

and returns a new memory state plus a next interval. Confirm exact types and
method names against the pinned crate version during implementation.

### 6.2 Algorithm version and parameters

Start with FSRS-6 default parameters and desired retention `0.90`.

Do not manually tune the 21 FSRS parameters. Store the parameter vector and its
provenance so defaults can later be replaced by optimized parameters.

Create a singleton configuration table:

```sql
CREATE TABLE scheduler_config (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  algorithm TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  desired_retention REAL NOT NULL
    CHECK (desired_retention >= 0.70 AND desired_retention <= 0.97),
  parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)),
  parameters_version INTEGER NOT NULL DEFAULT 1,
  parameters_source TEXT NOT NULL
    CHECK (parameters_source IN ('default', 'optimized')),
  timezone TEXT NOT NULL DEFAULT 'Europe/Madrid',
  day_cutoff_hour INTEGER NOT NULL DEFAULT 4
    CHECK (day_cutoff_hour BETWEEN 0 AND 23),
  updated_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

```

Insert row `id=1` during migration if absent. The Rust application should own
the authoritative default parameter vector; the migration may insert the same
serialized vector for transparency.

### 6.3 Time semantics

Persist review timestamps as UTC ISO-8601 strings. Convert them to learning-day
dates using the configured timezone and day cutoff when calculating `delta_t`.

For an existing memory state:

```text
elapsed_days = learning_day(reviewed_at) - learning_day(fsrs_last_review_at)

```

For a new state, pass `None` as the prior memory state and zero elapsed days.

Backdated imports must supply an explicit `reviewed_at`. Normal interactive
recording defaults to the current timestamp. A future session date without a
matching review timestamp must be rejected.

### 6.4 Next due date

After applying a rating:

1. Take the interval returned by the pinned `fsrs` crate for that rating.
2. Apply the crate's documented rounding/minimum behavior. Prefer the official
scheduling example's behavior rather than inventing custom rounding.
3. Store both the numeric scheduled interval and computed `due_at` in the review
log.
4. Update the cached current state on `weaknesses` in the same transaction.

`due_at` is a cache for bounded queue queries. The append-only review log and
stored parameters are the audit trail.

### 6.5 Delayed reviews

Do not punish or manually cap an overdue review. Pass the real elapsed days to
FSRS. Retrievability will be lower; FSRS accounts for whether delayed recall
ultimately succeeded or failed.

### 6.6 Personalized optimization

Do not implement optimization in the initial release. Preserve all rating and
timing history so it can be added later using `fsrs-rs`'s optimizer.

When optimization is eventually added:

* train from histories across many weaknesses;
* retain the previous parameter vector and version;
* evaluate fit before activating new parameters;
* recompute cached memory states deterministically from the review log;
* never mix review histories from different learners;
* never train from raw observations that were not accepted as FSRS reviews.

## 7. Database schema

The following schema is the intended end state. Exact migration syntax may be
adapted for SQLite compatibility, but field meanings must remain stable.

### 7.1 Weakness FSRS state

Add nullable columns to `weaknesses`:

```sql
ALTER TABLE weaknesses ADD COLUMN fsrs_stability REAL;
ALTER TABLE weaknesses ADD COLUMN fsrs_difficulty REAL;
ALTER TABLE weaknesses ADD COLUMN fsrs_due_at TEXT;
ALTER TABLE weaknesses ADD COLUMN fsrs_last_review_at TEXT;
ALTER TABLE weaknesses ADD COLUMN fsrs_algorithm_version TEXT;
ALTER TABLE weaknesses ADD COLUMN fsrs_parameters_version INTEGER;

```

Add constraints in application validation if SQLite cannot add them safely to
an existing table:

* stability must be positive when present;
* difficulty must be within `[1, 10]` when present;
* the state columns should be either consistently populated or consistently
null for an unreviewed weakness.

Retain the legacy scheduler columns for compatibility and migration audit, but
stop updating them once FSRS is enabled. Clearly mark them as legacy in comments
and outputs.

Create an index:

```sql
CREATE INDEX IF NOT EXISTS weaknesses_fsrs_queue_idx
ON weaknesses(active, fsrs_due_at);

```

### 7.2 Practice items

```sql
CREATE TABLE practice_items (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  item_no INTEGER NOT NULL CHECK (item_no >= 1),
  drill_type TEXT NOT NULL,
  prompt TEXT NOT NULL,
  response TEXT,
  corrected_response TEXT,
  reference_answer TEXT,
  feedback TEXT,
  outcome TEXT CHECK (
    outcome IS NULL OR outcome IN
      ('incorrect', 'partially_correct', 'correct', 'omitted')
  ),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, item_no)
);

CREATE INDEX practice_items_session_id_idx
ON practice_items(session_id);

```

`reference_answer` is optional because many prompts permit multiple natural
answers. `corrected_response` is the preferred correction of the learner's
actual answer. Do not treat `reference_answer` as the only acceptable answer.

### 7.3 Item targets

```sql
CREATE TABLE practice_item_targets (
  practice_item_id INTEGER NOT NULL
    REFERENCES practice_items(id) ON DELETE CASCADE,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  PRIMARY KEY (practice_item_id, weakness_id)
);

CREATE INDEX practice_item_targets_weakness_idx
ON practice_item_targets(weakness_id, practice_item_id);

```

This table records what an item was deliberately designed to test. It is not a
duplicate of observations: a target exists even when the learner makes no
observable error.

### 7.4 Attempts

Extend `attempts`:

```sql
ALTER TABLE attempts ADD COLUMN practice_item_id INTEGER
  REFERENCES practice_items(id) ON DELETE CASCADE;
ALTER TABLE attempts ADD COLUMN target_duration_seconds INTEGER;

```

*(Note: The existing `transcript` column remains untouched and continues to be used for storing response text in long-form exercises like 4/3/2).*

In the MCP request model, add `practice_item_no: Option<u16>` to an attempt.
Resolve it to `practice_item_id` after inserting the session's items. Keep it
optional so legacy session-level attempts remain readable and writable during
the compatibility period.

For a translation or completion item there is normally one item-level response
and no separate attempt row. For 4/3/2 there is normally one practice item (the
prompt) and three attempts linked to it, with target durations 240, 180, and 120.

`target_duration_seconds` describes the protocol. Do not add
`actual_duration_seconds`, pause count, filler count, or words per minute unless
the learner explicitly supplies such data in a future schema.

### 7.5 Observations

Extend observations with item and evidence semantics:

```sql
ALTER TABLE observations ADD COLUMN practice_item_id INTEGER
  REFERENCES practice_items(id) ON DELETE CASCADE;
ALTER TABLE observations ADD COLUMN role TEXT NOT NULL DEFAULT 'incidental';
ALTER TABLE observations ADD COLUMN evidence_strength TEXT;
ALTER TABLE observations ADD COLUMN severity TEXT;
ALTER TABLE observations ADD COLUMN error_span TEXT;

```

Validate these enums in Rust and, for newly created databases, with checks:

```text
role:
  targeted | incidental

evidence_strength:
  recognition | cued_production | controlled_production |
  spontaneous_production

severity:
  minor | meaning_affecting | blocking

```

The existing outcome values remain valid. Add `partially_correct` to the
observation outcome enum and database check for new databases. Migrating a
SQLite check constraint may require rebuilding the table transactionally.

### 7.6 FSRS review log

```sql
CREATE TABLE weakness_reviews (
  id INTEGER PRIMARY KEY,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  reviewed_at TEXT NOT NULL,
  rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
  retrieval_mode TEXT NOT NULL,
  evidence_strength TEXT NOT NULL,
  evidence_json TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(evidence_json)),
  elapsed_days INTEGER NOT NULL CHECK (elapsed_days >= 0),
  desired_retention REAL NOT NULL,
  retrievability_before REAL,
  stability_before REAL,
  difficulty_before REAL,
  stability_after REAL NOT NULL,
  difficulty_after REAL NOT NULL,
  scheduled_interval_days INTEGER NOT NULL
    CHECK (scheduled_interval_days >= 1),
  due_at TEXT NOT NULL,
  algorithm TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  parameters_version INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, weakness_id)
);

CREATE INDEX weakness_reviews_weakness_time_idx
ON weakness_reviews(weakness_id, reviewed_at);

```

`evidence_json` should be small and structured, for example:

```json
{
  "targeted_items": 2,
  "independent_correct": 1,
  "partially_correct": 0,
  "incorrect": 1,
  "final_attempt_correct": true,
  "rating_reason": "One independent success and one incorrect production"
}

```

Do not store arbitrary hidden chain-of-thought. `rating_reason` is a short,
user-visible justification.

### 7.7 Recorded Requests (Idempotency)

Ensure client-generated UUIDs are respected and payload integrity is guaranteed:

```sql
CREATE TABLE recorded_requests (
  idempotency_key TEXT PRIMARY KEY,
  request_hash TEXT NOT NULL,
  response_json TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

```

## 8. Drill taxonomy

Use a bounded enum in the MCP schema while storing the snake-case string:

```text
translation
situational_response
sentence_transformation
question_answer
sentence_completion
sentence_combining
error_correction
minimal_pair_choice
dialogue_completion
micro_story
retell
fluency_4_3_2

```

Suggested applicability:

| Weakness shape | Suitable drill types |
| --- | --- |
| Inflection/agreement | completion, transformation, translation |
| Choice/contrast | minimal-pair choice, error correction, situational response |
| Word order/clitics | transformation, sentence combining, translation |
| Collocation/fixed phrase | completion, dialogue completion, situational response |
| Tense/aspect | transformation, micro-story, retell |
| Fluency/discourse | micro-story, retell, 4/3/2 |

Add optional JSON or normalized metadata to weaknesses for
`recommended_drill_types`. Do not attempt to infer this mapping from the
weakness description on every request. Existing weaknesses can be backfilled by
a one-time reviewed migration or can use category-level defaults.

Recognition success is weaker evidence than production success. A recognition
item may contribute to the evidence summary, but recognition success alone must
not automatically become a `good` review of productive ability.

## 9. MCP tool design

Keep the public surface small. Add `get_practice_brief`; evolve
`record_practice_session`, `get_review_queue`, `get_recent_practice`, and
`get_learning_context`. Keep `upsert_weakness` and `get_data_status`.

### 9.1 `get_practice_brief`

This is a read-only tool. It selects targets and supplies context. It does not
write a planned session and does not generate the exercise wording.

Request model:

```json
{
  "count": 6,
  "objective": "review_due",
  "level": "A2",
  "drill_mix": "auto",
  "allowed_drill_types": null,
  "weakness_keys": null,
  "categories": null,
  "include_upcoming": true,
  "recent_prompts_per_weakness": 4,
  "as_of": null
}

```

Validation/defaults:

* `count`: default 6, range 1-20.
* `objective`: `review_due`, `targeted`, or `explore`; default `review_due`.
* `level`: optional bounded text; default `A2` for current single learner.
* `drill_mix`: `auto`, `production_focused`, `translation_only`,
`recognition_to_production`, `fluency`, or `custom`; default `auto`.
* `allowed_drill_types`: optional nonempty list from the drill enum, maximum 12.
* `weakness_keys`: optional explicit override, maximum 20.
* `categories`: optional filter, maximum 10.
* `recent_prompts_per_weakness`: default 4, range 0-10.
* `as_of`: optional valid `YYYY-MM-DD`; defaults to the current learning day.

Selection algorithm for `review_due`:

1. Load active weaknesses matching explicit filters.
2. Compute current retrievability from stored FSRS state and elapsed days. An
unreviewed weakness has no retrievability and is treated as due/new.
3. Partition candidates into due (`fsrs_due_at <= as_of`), new/unreviewed, and
upcoming.
4. Sort due candidates by:
* oldest due time first **(ensure queries use `ORDER BY fsrs_due_at ASC NULLS LAST` so `NULL` due dates of unreviewed items do not systematically precede heavily overdue items)**;
* lower current retrievability first;
* more recent `again` ratings first;
* least recently deliberately practised first;
* stable weakness key as final tie-breaker.


5. Fill from due candidates first, then new candidates, then upcoming candidates
only if `include_upcoming=true` and more targets are needed.
6. Apply diversity as a soft rule: avoid filling the whole batch with one
category when equally urgent alternatives exist. Never skip a materially
more overdue weakness merely to create cosmetic variety.
7. Allocate exercise opportunities across selected weaknesses. Default to two
opportunities per weakness for controlled batches, so six exercises usually
select three weaknesses. If the existing worst-outcome scheduler is still
active, do not enable this allocation; complete the FSRS recording migration
first.
8. Suggest drill families from weakness metadata and the requested drill mix.
9. Fetch the most recent distinct prompts targeting each selected weakness.
10. Return a generation brief. Do not create or reserve a session.

Explicit `weakness_keys` take precedence over automatic selection. Still return
FSRS state and recent prompts for those keys.

Use this deterministic allocation rule for a controlled batch:

1. Select up to `ceil(count / 2)` target weaknesses.
2. Give every selected target one opportunity.
3. Allocate remaining opportunities round-robin in selected-target order.
4. Prefer a maximum of two opportunities per target when enough eligible
targets exist. If there are too few eligible targets, distribute the remaining allocations by sorting eligible targets by priority and incrementing their allocations by 1 sequentially until the total equals `count`.

For `drill_mix=fluency`, default `count` to 1 when it was omitted and return one
4/3/2 prompt brief. Reject an explicit count other than 1 in the initial
implementation; `count` refers to prompts, not the three attempts.

Drill-mix meanings:

| Mix | Returned recommendation |
| --- | --- |
| `auto` | Suitable mix from the selected weaknesses' metadata; favor production. |
| `production_focused` | Only cued, controlled, or spontaneous production drills. |
| `translation_only` | All opportunities recommend `translation`. |
| `recognition_to_production` | Recognition may introduce a contrast, but every target also receives production evidence. |
| `fluency` | One `fluency_4_3_2` prompt with optional targeted weaknesses. |
| `custom` | Use only `allowed_drill_types`; require that field. |

Response shape:

```json
{
  "as_of": "2026-08-29",
  "objective": "review_due",
  "count": 6,
  "level": "A2",
  "targets": [
    {
      "weakness_key": "clitic-order",
      "category": "clitics",
      "description": "Order indirect and direct object pronouns",
      "target_pattern": "me/te/se/nos/os + lo/la/los/las",
      "allocation": 2,
      "selection_reason": "due; predicted retrievability 0.86",
      "fsrs": {
        "due_at": "2026-08-28",
        "retrievability": 0.86,
        "stability": 4.2,
        "difficulty": 6.1,
        "last_rating": "again"
      },
      "recommended_drill_types": [
        "sentence_transformation",
        "translation",
        "situational_response"
      ],
      "recent_prompts_to_avoid": [
        "She gave it to me yesterday.",
        "Can you send them to her?"
      ],
      "recent_error_examples": [
        {
          "produced": "...",
          "correction": "..."
        }
      ]
    }
  ],
  "generation_rules": [
    "Generate exactly 6 exercises using the allocations above",
    "Do not reveal weakness keys or answers before the learner responds",
    "Avoid semantic reuse of recent prompts",
    "Use natural A2-level language",
    "Accept natural equivalent answers",
    "Assess and record each item separately"
  ]
}

```

The server may report exact duplicate prompts from history, but it must not
claim that two prompts are semantically novel. ChatGPT performs semantic novelty
judgment using `recent_prompts_to_avoid`.

### 9.2 Revised `record_practice_session`

Retain the current idempotency key and existing fields for compatibility. Add:

```json
{
  "idempotency_key": "client-generated-uuid",
  "reviewed_at": "2026-08-29T18:30:00+02:00",
  "exercise_type": "mixed_drill_batch",
  "topic": "targeted review",
  "notes": null,
  "new_weaknesses": [
    {
      "key": "new-key",
      "category": "prepositions",
      "description": "...",
      "target_pattern": "...",
      "active": true
    }
  ],
  "items": [
    {
      "item_no": 1,
      "drill_type": "sentence_transformation",
      "prompt": "Replace 'the book' with an object pronoun: ...",
      "response": "...",
      "corrected_response": "...",
      "reference_answer": null,
      "feedback": "...",
      "outcome": "partially_correct",
      "target_weakness_keys": ["clitic-order"],
      "observations": [
        {
          "weakness_key": "clitic-order",
          "role": "targeted",
          "evidence_strength": "controlled_production",
          "severity": "meaning_affecting",
          "outcome": "incorrect",
          "produced": "...",
          "correction": "...",
          "error_span": "...",
          "notes": null
        }
      ]
    }
  ],
  "attempts": [],
  "observations": [],
  "reviews": [
    {
      "weakness_key": "clitic-order",
      "rating": "hard",
      "retrieval_mode": "controlled_production",
      "evidence_strength": "controlled_production",
      "evidence": {
        "targeted_items": 2,
        "independent_correct": 1,
        "incorrect": 1,
        "rating_reason": "One success and one incorrect production"
      }
    }
  ]
}

```

Recording transaction order:

1. Validate the full payload size, enum values, unique item/attempt numbers,
dates, and internal references before opening the write transaction where
possible. Compute a cryptographic hash of the entire request payload.
2. Begin a transaction.
3. Check `recorded_requests`. On idempotent replay (key exists), verify that the stored `request_hash` matches the newly computed payload hash (reject with error if the same key is reused for a different payload). Return the original session result without applying FSRS again.
4. Upsert `new_weaknesses` in the transaction.
5. Verify that every referenced weakness key now exists.
6. Insert the session.
7. Insert practice items and their target mappings.
8. Insert attempts and observations, resolving item numbers to IDs.
9. Validate every requested FSRS review:
* key exists and is active;
* only one review per weakness in this session;
* the weakness is a declared item target unless an explicit future override
is introduced;
* `reviewed_at` is not earlier than that weakness's last review;
* evidence is bounded JSON.


10. Load scheduler configuration and cached weakness memory state.
11. Calculate elapsed days and retrievability before review.
12. Call the official FSRS library for the supplied rating.
13. Insert `weakness_reviews` and update the weakness cached FSRS state.
14. Update weakness `first_seen`/`last_seen` from all observations and targets.
15. Insert into `recorded_requests` (storing the `idempotency_key`, `request_hash`, and generated `response_json`) and commit.

Any failure rolls back the session, newly created weaknesses, raw evidence, and
FSRS updates together.

Return:

```json
{
  "session_id": 42,
  "recorded": true,
  "idempotent_replay": false,
  "item_count": 6,
  "attempt_count": 0,
  "observation_count": 8,
  "new_weaknesses_created": ["new-key"],
  "review_updates": [
    {
      "weakness_key": "clitic-order",
      "rating": "hard",
      "retrievability_before": 0.86,
      "stability_before": 4.2,
      "stability_after": 4.8,
      "difficulty_before": 6.1,
      "difficulty_after": 6.3,
      "scheduled_interval_days": 2,
      "due_at": "2026-08-31"
    }
  ]
}

```

Do not return internal FSRS parameter arrays on every recording response.

### 9.3 4/3/2 recording example

Represent the exercise as one prompt item with three attempts:

```json
{
  "items": [
    {
      "item_no": 1,
      "drill_type": "fluency_4_3_2",
      "prompt": "Describe a journey where something unexpected happened.",
      "response": null,
      "target_weakness_keys": ["past-tense-aspect"],
      "observations": []
    }
  ],
  "attempts": [
    {
      "attempt_no": 1,
      "practice_item_no": 1,
      "target_duration_seconds": 240,
      "transcript": "...",
      "observations": []
    },
    {
      "attempt_no": 2,
      "practice_item_no": 1,
      "target_duration_seconds": 180,
      "transcript": "...",
      "observations": []
    },
    {
      "attempt_no": 3,
      "practice_item_no": 1,
      "target_duration_seconds": 120,
      "transcript": "...",
      "observations": []
    }
  ],
  "reviews": [
    {
      "weakness_key": "past-tense-aspect",
      "rating": "hard",
      "retrieval_mode": "spontaneous_production",
      "evidence_strength": "spontaneous_production",
      "evidence": {
        "final_attempt_correct": true,
        "rating_reason": "Initially incorrect but resolved in the final round"
      }
    }
  ]
}

```

Attempts remain at session level for compatibility with the existing request
shape. `practice_item_no` links each attempt to the prompt item.

Do not calculate actual duration, WPM, pause count, or filler count. It is valid
to calculate transcript character/word counts for display, but name them
`transcript_word_count`; never imply a rate or acoustic measurement.

### 9.4 `get_review_queue`

Keep the tool but change its basis to FSRS.

Each item should include:

```json
{
  "key": "clitic-order",
  "category": "clitics",
  "description": "...",
  "target_pattern": "...",
  "due_at": "2026-08-28",
  "is_due": true,
  "days_overdue": 1,
  "retrievability": 0.86,
  "stability": 4.2,
  "difficulty": 6.1,
  "last_reviewed_at": "2026-08-24T18:00:00Z",
  "last_rating": "again",
  "review_count": 5
}

```

Order due items using the same ordering as `get_practice_brief`. Upcoming items
follow by due time when requested. Unreviewed active weaknesses are due/new and
must be labeled as such rather than given fake retrievability.

### 9.5 Retrieval tools

Extend `get_recent_practice` to accept exact filters instead of relying only on
the overloaded `skill` substring:

* `exercise_types`
* `drill_types`
* `weakness_keys`
* `categories`
* `from_date`
* `to_date`
* `detail`: `summary` or `full`
* `include_items`
* `include_attempts`
* `include_observations`
* `include_reviews`

Default to a bounded summary. `full` may include prompts and transcripts but
must retain the current response-size protections.

Extend `get_learning_context` with a purpose/detail mode or at minimum include
compact FSRS fields and recent item prompts. Do not hard-code the weakness limit
to 12 without exposing a bounded request parameter.

### 9.6 `upsert_weakness`

Keep the existing single-item tool for manual maintenance. Recording's
`new_weaknesses` array handles atomic discoveries during a session.

New weaknesses have null FSRS state and are treated as due/new until their first
accepted review. Do not initialize fake stability or difficulty before the
first rating.

### 9.7 `get_data_status`

Increment the schema version and return:

```json
{
  "schema_version": 3,
  "scheduler": {
    "algorithm": "fsrs",
    "algorithm_version": "FSRS-6",
    "desired_retention": 0.90,
    "parameters_version": 1,
    "parameters_source": "default"
  },
  "counts": {
    "sessions": 0,
    "practice_items": 0,
    "attempts": 0,
    "observations": 0,
    "weaknesses": 0,
    "active_weaknesses": 0,
    "weakness_reviews": 0
  }
}

```

## 10. Exact and semantic novelty

Implement only deterministic support:

* store every practice item's exact prompt;
* return the last N distinct prompts for each selected weakness;
* optionally store a normalized prompt hash for exact duplicate checks;
* allow a duplicate prompt only when the caller explicitly marks it as
intentional repetition in a future extension.

A simple normalization may lowercase, trim, collapse whitespace, and normalize
Unicode. Do not remove content words or attempt semantic equivalence.

ChatGPT is responsible for reading `recent_prompts_to_avoid` and generating a
semantically different context. The server must not advertise semantic novelty
unless an embeddings/LLM feature is added later.

## 11. Migration strategy

Implement migrations transactionally and make them safe to rerun.

### Phase A: additive schema

1. Add FSRS state columns.
2. Create `scheduler_config`, `practice_items`, `practice_item_targets`, `weakness_reviews`, and `recorded_requests`.
3. Add nullable attempt/item and observation/item fields.
4. Rebuild `observations` only if necessary to extend its check constraint.
5. Preserve all IDs and foreign-key relationships.

### Phase B: historical FSRS reconstruction

Replay legacy observations grouped by `(weakness_id, session_id)` in session
date order. Use one synthetic timestamp per session derived from `session_date`
and the configured timezone. Record that timestamps were migrated in
`evidence_json`.

Map legacy evidence conservatively:

```text
all correct                       -> good (3)
mix of correct and incorrect      -> hard (2)
all incorrect or omitted          -> again (1)
any prompted_correct, no incorrect -> hard (2)

```

When a session has both prompted and correct outcomes, use `hard`. Never infer
`easy` from historical data.

Historical observations did not distinguish targeted from incidental. Mark
reconstructed reviews with:

```json
{
  "migrated": true,
  "targeting_unknown": true,
  "rating_reason": "Conservative mapping from legacy observations"
}

```

This reconstruction is imperfect but preferable to inventing precise targeting
information. Make it possible to skip reconstruction via an explicit migration
configuration if validation shows implausible schedules; in that case initialize
states using the official crate's SM-2 migration API from the legacy ease and
interval fields and clearly mark the provenance.

### Phase C: activate FSRS

Only switch queue and recording logic to FSRS after:

* reconstruction completes successfully;
* every populated cached state matches the latest review-log state;
* due dates are valid;
* idempotent replay tests prove reviews are not duplicated.

Retain legacy columns but stop mutating them.

## 12. Validation and invariants

Enforce these invariants in Rust even if SQLite also enforces them:

* One review per `(session, weakness)`.
* Every FSRS-reviewed weakness is a declared target in that session.
* Incidental observations alone never update FSRS.
* `easy` is accepted if explicitly supplied but never silently derived by the
server.
* `reviewed_at` is monotonic per weakness.
* A new weakness has either no FSRS state or a state created by its first review.
* Cached weakness state equals the newest review-log state.
* Idempotent replay never creates another session or review, and strictly enforces payload hash parity.
* A failed transaction leaves no partial weakness, item, observation, or review.
* Item and attempt numbers are unique in their intended scope.
* All text and array sizes remain bounded.
* No prompt or transcript is written to logs.

## 13. Tests

### 13.1 Unit tests

* New weakness + each rating produces the exact state/interval returned by the
pinned FSRS crate.
* Subsequent reviews use correct elapsed learning days.
* Overdue success and overdue failure pass real elapsed days to FSRS.
* Same-day elapsed days are handled according to FSRS-6.
* `easy` is not inferred anywhere in server/database code.
* Queue retrievability matches the crate for stored state and elapsed days.
* New/unreviewed weaknesses appear as due/new without fake retrievability.
* Mixed observations do not alter FSRS unless a review object is supplied.
* Incidental weakness observations do not update state.
* Two items for one weakness create one review.
* Three 4/3/2 attempts create one review.
* Target durations store 240/180/120 without actual-duration fields.

### 13.2 Transaction tests

* Inline weakness creation plus successful session recording commits all rows.
* Unknown weakness not present in `new_weaknesses` rolls everything back.
* Invalid review rating rolls everything back.
* Duplicate review for one weakness rolls everything back.
* Review timestamp older than previous review rolls everything back.
* Idempotent replay returns the original result and leaves review count unchanged.
* Idempotent replay with a mismatched payload hash is rejected.

### 13.3 Migration tests

* Upgrade a legacy database without losing rows.
* Replaying the migration twice is harmless.
* Legacy all-correct, mixed, and all-incorrect groups map to the documented
ratings.
* Cached state matches a clean replay of the resulting review history.
* Foreign-key check passes after table rebuilds.

### 13.4 MCP protocol tests

* Tool list contains the new tool and updated JSON schemas.
* All enums appear in schemas.
* Unknown fields remain rejected.
* `get_practice_brief` is annotated read-only.
* Recording remains non-destructive and idempotent.
* Representative translation, mixed drill, and 4/3/2 payloads round-trip.
* Bounded limits and maximum payload sizes are enforced.

### 13.5 Selection tests

* Due targets outrank upcoming targets.
* Lower retrievability breaks ties as specified.
* Explicit weakness keys override automatic selection.
* Recent prompts returned are actually linked to the selected weakness.
* Diversity never displaces a materially more overdue candidate.
* Output ordering is deterministic for a fixed database and `as_of`.

## 14. Implementation order

Implement in this order so intermediate commits remain reviewable:

1. Add and test FSRS crate integration behind an internal adapter module.
2. Add schema migrations and data types without changing public scheduling.
3. Implement historical reconstruction and state-consistency checks.
4. Replace queue calculations with FSRS.
5. Add practice items, item targets, and enriched observations.
6. Evolve `record_practice_session` and add atomic inline weaknesses/reviews.
7. Add `get_practice_brief` and deterministic selection tests.
8. Extend recent/context/status outputs.
9. Update protocol, smoke, migration, and README documentation.
10. Run `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`, and the smoke
test.

Do not release unless explicitly requested. The repository's release command is
`scripts/release.sh`.

## 15. Acceptance examples

### Six-item mixed batch

1. ChatGPT calls `get_practice_brief(count=6, drill_mix=auto)`.
2. MCP returns three due weaknesses, two opportunities each, recommended drill
families, and four recent prompts per weakness.
3. ChatGPT writes six novel exercises and presents them without weakness labels
or answers.
4. Learner responds.
5. ChatGPT corrects each item, suggests one rating per targeted weakness, and
lets the learner override ratings.
6. ChatGPT calls `record_practice_session` with all six items, raw observations,
inline new weaknesses, and three reviews.
7. MCP atomically stores the session and applies FSRS three times.
8. Response shows next due dates and short rating evidence.

### 4/3/2 session

1. ChatGPT requests a fluency practice brief.
2. MCP returns a prompt, recent prompts to avoid, and optional targeted
weaknesses.
3. Learner uses an external timer and supplies three transcripts.
4. ChatGPT compares content retained, errors resolved, and persistent errors.
5. ChatGPT does not claim actual duration, WPM, or pause metrics.
6. ChatGPT suggests at most one FSRS rating for each deliberately targeted
weakness; incidental errors are observations only.
7. MCP stores one item, three attempts, all observations, and one review per
targeted weakness.

## 16. Future work, deliberately excluded

* Personalized FSRS parameter optimization and evaluation.
* Separate FSRS tracks by retrieval mode.
* Embedding-based semantic prompt novelty.
* User-editable prompt banks.
* Recurring calendar cadence for general fluency practice.
* Audio-derived timing and acoustic fluency metrics.
* DELE A2 task schemas, rubrics, media, and mock-exam planning.
