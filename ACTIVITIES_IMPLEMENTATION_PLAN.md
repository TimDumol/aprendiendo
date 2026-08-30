# Activity Support Implementation Plan

## 1. Purpose and authority

This document specifies the next Aprendiendo data-model and MCP revision needed
to support the activities in `ACTIVITIES.md`. It is written for an
implementation agent that has no context beyond this repository and this file.
Treat decisions marked **Decision** as settled unless repository constraints
make them technically impossible.

Read `FSRS_PLAN.md` and `TAXONOMY_PLAN.md` before implementing this plan. Those
documents continue to control FSRS ratings, evidence integrity, historical data,
and migration safety. This plan controls activity planning, activity recording,
stimulus representation, transcript-only timing metadata, and activity
reporting.

No production release is part of this work unless explicitly requested. When a
release is requested, use `scripts/release.sh`.

## 2. Critical input limitation

**Decision:** ChatGPT receives text transcripts, not the underlying learner
audio. The MCP server must treat a spoken answer as a transcript of speech, not
as an audio recording.

The implementation must not:

- accept or store audio blobs;
- claim that audio was analyzed;
- infer speaking duration from transcript length;
- infer response latency, pause count, pause duration, speech rate, fillers,
  pronunciation, prosody, or acoustic fluency from a transcript;
- infer that a response was effortless because its transcript is correct;
- label textual punctuation or transcription artifacts as learner pauses.

The implementation may store:

- the transcript supplied to ChatGPT;
- whether the answer was typed or was a transcript of speech;
- a target duration chosen before the task;
- an actual duration or response latency only when supplied by the learner, an
  external timer, or a client that measured it;
- learner-reported hesitations, simplifications, retrieval gaps, and
  self-corrections;
- the textual transcript or description of a source that the learner reports
  reading, hearing, or viewing.

Every measured/reported timing value added by this plan must carry a source.
Missing timing data is valid and must remain `NULL`.

## 3. Goals

- Make the activity being practiced explicit and queryable.
- Make `get_practice_brief` honor its drill-selection inputs instead of merely
  echoing them.
- Plan the ten activities in `ACTIVITIES.md` without putting language
  generation into Rust.
- Group multi-item activities such as Q&A sprints, conversations, and role-play
  turns while retaining per-item assessment.
- Represent source text, image descriptions, reported media transcripts, and
  reveal/hide constraints without storing binary media.
- Store learner reflections that are useful but are not linguistic weakness
  observations.
- Keep existing sessions, items, attempts, observations, reviews, and
  idempotency behavior intact.
- Preserve compatibility for callers that do not yet send activity fields.
- Keep all MCP requests and responses bounded.

## 4. Non-goals

- Audio upload, storage, transcription, or analysis.
- Image upload or binary image storage in SQLite.
- Automatic pronunciation assessment.
- Automatic detection of pauses, hesitations, speech rate, or time to first
  word.
- Fetching article, audio, video, or image URLs from the MCP server.
- Natural-language prompt generation in Rust.
- A generic workflow engine.
- Scheduling the habit of doing an activity. FSRS continues to schedule
  learning targets.
- Splitting existing FSRS state into recognition, controlled-production, and
  spontaneous-production tracks in this release.
- Reclassifying or deleting historical sessions.

## 5. Current implementation facts

The current target schema version is 7. `data/aprendiendo-live.sqlite3` is the
asserted schema-3 production snapshot used by the existing migration chain. A
new migration must upgrade both a schema-7 database and an exact schema-3 copy
that first passes through migrations 4 through 7.

Useful existing structures:

- `sessions` records a coarse session-level `exercise_type_key`.
- `practice_items` records one prompt, response, feedback, outcome, and one
  `drill_type` per item.
- `attempts` records transcripts and optional target/actual duration.
- `observations` already distinguishes targeted/incidental evidence, assessment
  phase, hints, and evidence strength.
- `scheduler_items` contains possible track names, but current code deliberately
  reads and updates only `track='general'`.
- `record_practice_session` is atomic and idempotent.

Important current defect:

- `PracticeBriefRequest` accepts `drill_mix` and `allowed_drill_types`, but
  `SqliteStore::practice_brief` does not use `allowed_drill_types` and only
  special-cases `DrillMix::Fluency` by forcing `count=1`.

Fix that defect before adding automatic activity selection.

## 6. Domain model decisions

### 6.1 Session, activity run, item, and attempt

Use these meanings consistently:

- A **session** is one recorded practice event on one learning day.
- An **activity run** is one execution of an activity within a session, such as
  one six-question sprint or one pharmacy role-play.
- A **practice item** is one learner-facing prompt or turn assessed separately.
- An **attempt** is one learner response transcript for an item.
- A **stimulus** is source/context material for the whole activity run.
- A **reflection** is a learner- or assistant-reported process note that is not
  itself an observation of a stable learning target.

Do not replace any existing table. Add `activity_runs`, `activity_stimuli`, and
`attempt_reflections`, then link new practice items to an activity run.

### 6.2 Activity type versus drill type

**Decision:** Activity type and drill type are different.

Activity type describes the learner experience and orchestration. Drill type
describes the cognitive task of an individual practice item. Keep the existing
`DrillType` enum and SQL checks unchanged in this migration.

Use these activity keys and compatible existing drill types:

| Activity key | Label | Compatible drill types | Default item count |
| --- | --- | --- | ---: |
| `situational_response` | Situational response | `situational_response` | 4 |
| `picture_narration` | Picture/comic narration | `micro_story`, `retell` | 1 |
| `question_answer_sprint` | Question-answer sprint | `question_answer` | 6 |
| `retell_reconstruction` | Retell/reconstruction | `retell` | 1 |
| `corrective_conversation` | Conversation with corrective feedback | `question_answer`, `situational_response`, `dialogue_completion` | 6 |
| `sentence_transformation_sprint` | Sentence-transformation sprint | `sentence_transformation` | 6 |
| `dictogloss` | Dictogloss | `retell` | 1 |
| `voice_diary` | Voice diary | `situational_response`, `micro_story` | 1 |
| `read_close_explain` | Read, close, explain | `retell` | 1 |
| `role_play_complications` | Role-play with complications | `situational_response`, `dialogue_completion`, `question_answer` | 4 |

Do not add activity keys to the `DrillType` enum. Do not label a transcript as
audio merely because the activity was spoken.

Implement one Rust `ActivitySpec` catalog, keyed by `ActivityType`, containing
the label, interaction mode, default count, compatible drills, default config,
stimulus requirements, and intended evidence strength. Brief generation,
recording validation, and the `activity_types` seed helper must consume this
catalog rather than maintaining independent match statements. SQL `CHECK`
lists still remain explicit schema constraints. Add a test that the catalog has
exactly one entry for every `ActivityType` variant and that its keys, labels,
and interaction modes exactly match the ten seeded database rows.

### 6.3 Coarse exercise type

Keep `sessions.exercise_type_key` for compatibility. New activity-aware callers
should normally use:

- `production_drill` for single-response and sprint activities;
- `guided_conversation` for corrective conversation and role-play;
- `fluency_4_3_2` for existing 4-3-2 sessions.

The activity run is the authoritative fine-grained classification. Do not seed
ten duplicate session exercise types.

### 6.4 Scheduler behavior

**Decision:** Keep one active `general` scheduler item per weakness in this
release, consistent with `FSRS_PLAN.md` and the sparse historical dataset.

Continue storing `retrieval_mode` and `evidence_strength` on review evidence.
Activity briefs should request spontaneous-production evidence where
appropriate, but the activity migration must not create, initialize, merge, or
backfill mode-specific FSRS states.

A future migration may split tracks after enough deliberate production reviews
exist. Historical `general` state must not later be relabeled as spontaneous
mastery without explicit policy and tests.

## 7. Schema version 8

Add migration 8 named `008_activity_runs`. Use one checksum constant following
the existing migration registry pattern.

**Decision:** The migration registry row and `schema_meta.schema_version='8'`
must be written in the same transaction as the schema changes. Before writing
either marker, run the migration-8 pre-commit checks described in section 14
against that transaction. Commit only after those checks succeed. Run the
existing external postflight again after commit as defense in depth. A crash
must never leave migration 8 registered while `schema_meta` still reports 7.

Add the same final table and column definitions to `sql/sqlite_schema.sql` so a
new empty database and a migrated database have the same schema.

### 7.1 `activity_types`

```sql
CREATE TABLE activity_types (
  key TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  interaction_mode TEXT NOT NULL CHECK (interaction_mode IN (
    'single_response', 'sprint', 'multi_turn'
  )),
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);
```

Seed all ten rows listed in section 6.2. Use:

- `single_response`: picture narration, retell/reconstruction, dictogloss,
  voice diary, and read-close-explain;
- `sprint`: situational response, Q&A sprint, and sentence transformation;
- `multi_turn`: corrective conversation and role-play with complications.

Use `INSERT OR IGNORE` in a shared seed helper invoked by both empty-database
initialization and migration 8.

### 7.2 `activity_runs`

```sql
CREATE TABLE activity_runs (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  run_no INTEGER NOT NULL CHECK (run_no >= 1),
  activity_type_key TEXT NOT NULL REFERENCES activity_types(key),
  planned_duration_seconds INTEGER
    CHECK (planned_duration_seconds IS NULL OR planned_duration_seconds > 0),
  actual_duration_milliseconds INTEGER
    CHECK (actual_duration_milliseconds IS NULL OR actual_duration_milliseconds > 0),
  timing_source TEXT CHECK (timing_source IS NULL OR timing_source IN (
    'learner_reported', 'external_timer', 'client_measured'
  )),
  config_json TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(config_json) AND json_type(config_json)='object'),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, run_no),
  UNIQUE(id, session_id),
  CHECK (actual_duration_milliseconds IS NULL OR timing_source IS NOT NULL)
);

CREATE INDEX activity_runs_type_session_idx
  ON activity_runs(activity_type_key, session_id);
```

`config_json` is for bounded activity constraints, not arbitrary unvalidated
application state. The Rust request type defined in section 8 serializes into
this field.

### 7.3 Link items to runs

Add a nullable `activity_run_id` to `practice_items`:

```sql
activity_run_id INTEGER
```

Also add:

```sql
item_phase TEXT NOT NULL DEFAULT 'initial' CHECK (item_phase IN (
  'initial', 'follow_up', 'complication'
))
```

In the rebuilt table, add this composite foreign key so a practice item cannot
reference a run from another session:

```sql
FOREIGN KEY (activity_run_id, session_id)
  REFERENCES activity_runs(id, session_id) ON DELETE CASCADE
```

Legacy items retain `activity_run_id=NULL` and `item_phase='initial'`. New
activity-aware requests must link every item to a declared run. Do not attempt
to infer activity runs for historical items.

Add an index on `practice_items(activity_run_id, item_no)`.

Because SQLite cannot add the composite foreign key to an existing table,
rebuild `practice_items` using the safe table-rebuild approach already used by
the evidence migration. At schema version 7, `practice_items` is referenced by
`practice_item_targets`, `attempts`, and `observations`, and observations are in
turn referenced by `review_observations`. Rebuild or redirect all of those
dependent tables in the same transaction; do not drop `practice_items` while a
surviving table still references its old SQLite table identity. Preserve every
ID and column value, including review-to-observation IDs. Recreate all indexes,
unique constraints, foreign keys, and `observations_same_item_trigger`. Run
`foreign_key_check` before committing and again in external postflight.

Use this concrete rebuild order:

1. Create `practice_items_new`, `practice_item_targets_new`, `attempts_new`,
   `observations_new`, and `review_observations_new`, with foreign keys between
   the `_new` tables where applicable.
2. Copy every row with explicit source and destination column lists. Populate
   only the new columns with their documented legacy defaults.
3. Verify copied counts and the captured projections before dropping anything.
4. Drop old child tables in dependency order:
   `review_observations`, `observations`, `attempts`,
   `practice_item_targets`, then `practice_items`.
5. Rename new tables in parent-first order: `practice_items_new`,
   `practice_item_targets_new`, `attempts_new`, `observations_new`, then
   `review_observations_new`.
6. Create `attempt_reflections` only after the final `attempts` table exists.
7. Recreate indexes and the observation trigger, then run the pre-commit checks.

Do not use `PRAGMA foreign_keys=OFF`; it cannot be safely toggled inside an
active transaction and would weaken the migration's failure guarantees.

`item_phase` describes the conversational role of a prompt. Evidence conditions
such as guided practice, immediate retry, and transfer continue to belong only
to `observations.assessment_phase`; do not create a second copy of that evidence
taxonomy on practice items.

### 7.4 `activity_stimuli`

```sql
CREATE TABLE activity_stimuli (
  id INTEGER PRIMARY KEY,
  activity_run_id INTEGER NOT NULL REFERENCES activity_runs(id) ON DELETE CASCADE,
  stimulus_no INTEGER NOT NULL CHECK (stimulus_no >= 1),
  kind TEXT NOT NULL CHECK (kind IN (
    'situation', 'source_text', 'image_description',
    'image_sequence_description', 'article_reference',
    'media_transcript', 'complication'
  )),
  delivery_mode TEXT NOT NULL CHECK (delivery_mode IN (
    'read', 'viewed', 'heard_reported', 'conversation'
  )),
  content_text TEXT,
  source_uri TEXT,
  content_fingerprint TEXT,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(activity_run_id, stimulus_no),
  CHECK (
    (content_text IS NOT NULL AND length(trim(content_text)) > 0)
    OR (source_uri IS NOT NULL AND length(trim(source_uri)) > 0)
  )
);

CREATE INDEX activity_stimuli_run_idx
  ON activity_stimuli(activity_run_id, stimulus_no);
```

Rules:

- `image_description` and `image_sequence_description` store text descriptions,
  not image bytes.
- `media_transcript` stores only the transcript available to ChatGPT. It does
  not prove the learner heard the underlying media.
- `heard_reported` means the learner/client reports that the source was heard;
  it does not mean the MCP received audio.
- The server never dereferences `source_uri`.
- Compute `content_fingerprint` from normalized `content_text` when present;
  otherwise from the URI. Follow the current prompt-fingerprint approach.
- Cap `content_text` at 8,000 bytes and `source_uri` at 2,000 bytes.

Do not add an open-ended stimulus `metadata_json` field in version 8. Kind,
delivery mode, content/reference, fingerprint, and run-level typed config cover
the stated activities. Add a typed column in a later migration if a concrete
metadata requirement appears.

Application validation must enforce this compatibility matrix. Supplying both
text and a URI is allowed only for `article_reference`; all other kinds reject a
URI. Required text means non-empty after trimming.

| Kind | Allowed delivery mode | Required representation |
| --- | --- | --- |
| `situation` | `read`, `conversation` | `content_text` |
| `source_text` | `read` | `content_text` |
| `image_description` | `viewed` | `content_text` |
| `image_sequence_description` | `viewed` | `content_text` |
| `article_reference` | `read` | `content_text`, `source_uri`, or both |
| `media_transcript` | `heard_reported`, `viewed` | `content_text` |
| `complication` | `conversation` | `content_text` |

`media_transcript.content_text` is the transcript available to ChatGPT;
`heard_reported` or `viewed` remains only the caller's report about delivery.
A URI alone must never satisfy a transcript or visual-description requirement.

Normalize text exactly as `prompt_fingerprint` currently does: Unicode text is
lowercased, split on whitespace, and rejoined with one ASCII space before
SHA-256. For a URI-only article reference, trim the URI and hash its remaining
bytes without dereferencing or otherwise canonicalizing it.

### 7.5 Attempt transcript metadata

Add nullable fields to `attempts`:

```sql
response_mode TEXT CHECK (response_mode IS NULL OR response_mode IN (
  'typed', 'spoken_transcript'
)),
response_latency_milliseconds INTEGER CHECK (
  response_latency_milliseconds IS NULL OR response_latency_milliseconds > 0
),
timing_source TEXT CHECK (timing_source IS NULL OR timing_source IN (
  'learner_reported', 'external_timer', 'client_measured'
)),
CHECK (response_latency_milliseconds IS NULL OR timing_source IS NOT NULL)
```

Add `CREATE INDEX attempts_response_mode_session_idx ON
attempts(response_mode, session_id)` for the new recent-practice filter.

Application validation must require `timing_source` when either
`actual_duration_milliseconds` or `response_latency_milliseconds` is supplied by
an activity-aware request. Preserve legacy attempts that already have actual
duration but no timing source; do not rewrite or infer their source.

Use `spoken_transcript` only when the caller says the transcript came from a
spoken response. It still contains no audio evidence.

Use `NULL`, not a separate `unknown` enum value, when the response mode was not
supplied. This preserves one representation for unknown legacy data.

### 7.6 `attempt_reflections`

```sql
CREATE TABLE attempt_reflections (
  id INTEGER PRIMARY KEY,
  attempt_id INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
  reflection_no INTEGER NOT NULL CHECK (reflection_no >= 1),
  source TEXT NOT NULL CHECK (source IN ('learner', 'assistant')),
  kind TEXT NOT NULL CHECK (kind IN (
    'hesitation_reported', 'simplification_reported',
    'retrieval_gap_reported', 'self_correction_reported',
    'circumlocution_reported', 'general'
  )),
  note TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(attempt_id, reflection_no)
);

CREATE INDEX attempt_reflections_attempt_idx
  ON attempt_reflections(attempt_id, reflection_no);
```

Reflections do not require a weakness key and never update FSRS. If a reflection
reveals a stable lexical or grammatical target, record that separately through
the existing candidate-weakness and observation workflow.

`source` identifies the origin of the reported fact, not the process that wrote
the database row. All `*_reported` reflection kinds require `source='learner'`.
An assistant may store such a reflection on the learner's behalf, but must still
mark its source as `learner`. `source='assistant'` is permitted only with
`kind='general'`, for non-acoustic process notes directly supported by the text
interaction. Assistant reflections must not infer hesitation, pronunciation,
pauses, effort, or timing from transcript text.

## 8. Rust model changes

Add serializable `JsonSchema` enums with `snake_case` names:

- `ActivityType` with the ten keys in section 6.2;
- `ActivityItemPhase`;
- `StimulusKind`;
- `StimulusDeliveryMode`;
- `ResponseMode`;
- `TimingSource`;
- `ReflectionSource`;
- `ReflectionKind`.

Add these request structures with `#[serde(deny_unknown_fields)]`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityConfigInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_count: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_count: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hidden_before_response: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpredictable_followups: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complication_count: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityStimulusInput {
    pub stimulus_no: u16,
    pub kind: StimulusKind,
    pub delivery_mode: StimulusDeliveryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityRunInput {
    pub run_no: u16,
    pub activity_type: ActivityType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_duration_milliseconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_source: Option<TimingSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ActivityConfigInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stimuli: Vec<ActivityStimulusInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttemptReflectionInput {
    pub reflection_no: u16,
    pub source: ReflectionSource,
    pub kind: ReflectionKind,
    pub note: String,
}
```

Extend existing inputs:

- `PracticeBriefRequest.activity_type: Option<ActivityType>`
- `PracticeBriefRequest.planned_duration_seconds: Option<u32>`
- `PracticeBriefRequest.activity_config: Option<ActivityConfigInput>`
- `RecordPracticeSessionRequest.activity_runs: Vec<ActivityRunInput>` with
  `#[serde(default, skip_serializing_if = "Vec::is_empty")]`
- `PracticeItemInput.activity_run_no: Option<u16>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`
- `PracticeItemInput.item_phase: Option<ActivityItemPhase>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`
- `AttemptInput.response_mode: Option<ResponseMode>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`
- `AttemptInput.response_latency_milliseconds: Option<u64>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`
- `AttemptInput.timing_source: Option<TimingSource>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`
- `AttemptInput.reflections: Vec<AttemptReflectionInput>` with
  `#[serde(default, skip_serializing_if = "Vec::is_empty")]`

Keep all new recording fields optional/defaulted so old valid payloads continue
to deserialize. The `skip_serializing_if` annotations are required for
idempotency compatibility: the server reserializes the typed request before
hashing it, so absent version-8 fields must not become explicit `null` or `[]`
values and change the hash of an old replayable request.

Within the new nested activity structures, also skip absent optional fields
when serializing. Reject `config: {}` (or a config whose fields are all absent)
and require callers to omit `config` instead. Store an omitted run config as
the database value `{}`. These rules provide one canonical representation for
an empty config.

## 9. Input validation

Implement validation in `src/server.rs` before opening a write transaction and
repeat relational validation in `src/db.rs` where necessary.

### 9.1 General bounds

- At most 10 activity runs per session.
- At most 20 stimuli per run.
- At most 20 reflections per attempt.
- Brief and recorded `planned_duration_seconds`: 1 through 7,200.
- New activity-run actual durations: 1 through 86,400,000 milliseconds.
- Activity-aware attempt actual durations: 1 through 86,400,000 milliseconds.
- Response latency: 1 through 3,600,000 milliseconds.
- `preparation_seconds` and `response_seconds`: 1 through 3,600.
- `question_count`: 1 through 20.
- `exposure_count`: 1 through 10.
- `complication_count`: 0 through 10.
- Run and stimulus numbers must be positive and unique in their parent scope.
- Reflection numbers must be positive and unique within an attempt.
- Run notes: 4,000 bytes maximum.
- Reflection notes: 2,000 bytes maximum.
- Preserve the total 64 KiB serialized practice-session limit.

All new required text and all new optional text when present must be non-empty
after trimming. Do not tighten blank-string behavior for pre-version-8 fields,
because old valid recording payloads must retain their behavior. Continue
measuring limits in UTF-8 bytes, as the existing server validation does.
Validate stimulus kind/delivery/content combinations using the matrix in
section 7.4. Require `source='learner'` for every non-`general` reflection kind
as specified in section 7.6.

### 9.2 Referential validation

For activity-aware recording:

- Every `activity_run_no` on an item must reference a declared run.
- If any activity runs are declared, every practice item must have an
  `activity_run_no`.
- If any activity runs are declared, every attempt must reference a declared
  practice item through `practice_item_no`; activity-aware sessions must not
  contain orphan attempts.
- An item's drill type must be compatible with its activity type according to
  section 6.2.
- A run with actual duration must have a timing source.
- An attempt with response latency must have a timing source.
- For new activity-aware attempts, actual duration also requires a timing
  source.
- In an activity-aware request, reject a run or attempt `timing_source` when no
  corresponding actual duration or response latency is present.
- A single-response activity must contain exactly one practice item.
- A sprint or multi-turn activity must contain at least one practice item.
- `complication` item phase is only valid for `role_play_complications` or
  `corrective_conversation`.

Old requests with no `activity_runs` retain existing behavior and may contain
items with no activity run.

### 9.3 Activity-specific validation

- `picture_narration` requires an `image_description` or
  `image_sequence_description` stimulus.
- `retell_reconstruction` requires `source_text` or `media_transcript`.
- `dictogloss` requires `source_text` or `media_transcript`, an
  `exposure_count`, and `source_hidden_before_response=true`.
- `read_close_explain` requires `source_text` or `article_reference` and
  `source_hidden_before_response=true`.
- `role_play_complications` requires at least one `situation` stimulus. When
  `complication_count` is supplied, it must equal the number of linked items
  with phase `complication`. It must be smaller than the run's item count so at
  least one non-complication turn remains.
- `question_answer_sprint` must have `question_count` equal to the number of
  linked items when `question_count` is supplied.
- `voice_diary` accepts no required stimulus. Its transcript is stored on the
  attempt, not duplicated as stimulus text.

These checks validate declared structure only. They must not claim that the
learner actually closed a text, heard a recording, or began speaking within a
given time.

Reject config fields that do not apply to an activity rather than silently
storing them:

| Config field | Activities where accepted |
| --- | --- |
| `preparation_seconds` | situational response, picture narration, Q&A sprint |
| `response_seconds` | situational response, picture narration, Q&A sprint, retell/reconstruction, dictogloss, voice diary, read-close-explain |
| `question_count` | Q&A sprint |
| `exposure_count` | retell/reconstruction, dictogloss |
| `source_hidden_before_response` | retell/reconstruction, dictogloss, read-close-explain |
| `unpredictable_followups` | corrective conversation, role-play with complications |
| `complication_count` | role-play with complications |

An absent optional config field remains unknown unless brief defaults supplied
it. Recording validation must not silently apply brief defaults to a caller's
payload. In particular, an absent `complication_count` does not prove that a
role-play had a complication; a recorded complication is represented by an
item with `item_phase='complication'`. A separate complication stimulus is
optional; the practice-item prompt itself may contain the revealed
complication.

## 10. `get_practice_brief` behavior

### 10.1 Preserve responsibilities

The database selects targets, activity structure, drill families, timing
targets, and recent prompts to avoid. ChatGPT generates the actual Spanish
prompts and conducts the interaction.

Do not return generated exercises from Rust.

### 10.2 Count semantics

When `activity_type` is supplied:

- Force `count=1` for picture narration, retell/reconstruction, dictogloss,
  voice diary, and read-close-explain. Reject an explicit count other than 1.
- Interpret count as the number of prompts/learner turns for all other
  activities.
- Use the default count in section 6.2 when count is absent.

When no activity type is supplied, preserve the current default count of 6,
except for the existing fluency behavior.

`planned_duration_seconds` is the target duration for the whole activity run,
not a per-item response target. It is valid only when `activity_type` is
supplied. Per-item preparation and response targets are supplied through
`activity_config`. Reject `activity_config` without `activity_type`, and validate
its applicable fields using the matrix in section 9.3. Request values override
the defaults in section 10.5 field by field; omitted fields retain their
activity default.

Structural invariants still apply to overrides: dictogloss and
read-close-explain reject `source_hidden_before_response=false`; Q&A sprint
rejects a supplied `question_count` different from the effective `count`.
When Q&A `count` is overridden and `question_count` is omitted, set the default
`question_count` to that effective count rather than retaining six.

### 10.3 Make drill constraints effective

Implement one deterministic helper over a typed recommendation record containing
`drill_type`, `stage`, and `weight`. It receives:

- requested activity type, if any;
- requested `DrillMix`;
- `allowed_drill_types`, if any;
- database recommendations for one target.

It returns an ordered list of eligible drill recommendations. Do not perform
selection over `serde_json::Value`.

Before filtering target recommendations, validate the top-level combination:

- With an `activity_type`, allow only `auto`, `production_focused`, and
  `custom`.
- Reject `translation_only`, `recognition_to_production`, and `fluency` when an
  activity is supplied. The ten activities are production experiences and none
  declares `translation` or `fluency_4_3_2` as a compatible item drill.
- Without an activity, all existing drill mixes remain supported.

Apply filters in this order:

1. Start with database recommendations for the target.
2. If an activity is requested, retain only its compatible drill types. If that
   leaves none, use all compatible drill types from section 6.2 as activity
   fallbacks. Assign `controlled` stage to sentence transformation and dialogue
   completion; assign `transfer` to the other activity-compatible drills. Give
   each fallback weight 1.0. This fallback is required so an activity remains
   usable for a target whose taxonomy recommendations cover only other drill
   families.
3. Without an activity, use the following general fallback only when the target
   has no database recommendations: `minimal_pair_choice/recognition`,
   `error_correction/recognition`, `translation/controlled`,
   `sentence_transformation/controlled`, `sentence_completion/controlled`,
   `sentence_combining/controlled`, `situational_response/transfer`,
   `question_answer/transfer`, `dialogue_completion/controlled`,
   `micro_story/transfer`, `retell/transfer`, and
   `fluency_4_3_2/fluency`, all with weight 1.0.
4. If `allowed_drill_types` is present, intersect with it. Do not restore a
   fallback after this caller-supplied constraint makes the list empty.
5. Apply the drill mix:
   - `auto`: retain all remaining recommendations in weight order;
   - `production_focused`: retain `situational_response`,
     `sentence_transformation`, `question_answer`, `dialogue_completion`,
     `micro_story`, `retell`, `sentence_combining`, and
     `sentence_completion`;
   - `translation_only`: retain only `translation`;
   - `recognition_to_production`: require at least one recognition-stage and
     one controlled/transfer/fluency-stage recommendation;
   - `fluency`: retain only `fluency_4_3_2` and preserve count 1;
   - `custom`: require and use `allowed_drill_types`.
6. Sort the result by weight descending, drill key ascending, then stage
   ascending. If several database rows recommend the same `(drill_type, stage)`,
   keep the highest-weight row; break equal-weight provenance ties by shortest
   concept distance and then source concept key.
7. If the result is empty, return a validation/domain error that names the
   incompatible activity/mix constraints.

Do not silently ignore any requested constraint.

For `recognition_to_production` without an activity, every selected target must
receive at least two items. Allocate the first occurrence of each target to its
highest-ranked recognition recommendation and every later occurrence to its
highest-ranked controlled, transfer, or fluency recommendation. For an explicit
weakness list, reject the request when `count < 2 * weakness_count`; do not
silently omit an explicitly requested target. For automatic target selection,
select at most `floor(count / 2)` targets. For other mixes, select the
highest-ranked eligible recommendation for each occurrence.

The database recommendation loader must retain one row per `(drill_type,
stage)` after provenance deduplication. The current `HashMap::into_values()`
result is not sufficiently ordered and must not feed allocation directly.

### 10.4 Response shape

Retain existing top-level fields and target metadata. Add:

```json
{
  "activity_plan": {
    "activity_type": "question_answer_sprint",
    "interaction_mode": "sprint",
    "run_count": 1,
    "item_count": 6,
    "config": {
      "preparation_seconds": 3,
      "response_seconds": 30,
      "question_count": 6
    },
    "stimulus_requirements": [],
    "item_allocations": [
      {
        "item_no": 1,
        "drill_type": "question_answer",
        "target_weakness_keys": ["example_key"],
        "phase": "initial",
        "target_duration_seconds": 30,
        "intended_evidence_strength": "spontaneous_production"
      }
    ],
    "recording_rules": [
      "Store each answer as a transcript linked to its item",
      "Do not infer timing or acoustic fluency from transcript text"
    ]
  }
}
```

`stimulus_requirements` is a bounded array of declarative requirement objects,
not generated stimulus content. Each object has this exact shape:

```json
{
  "minimum_count": 1,
  "alternatives": [
    {
      "kind": "source_text",
      "allowed_delivery_modes": ["read"],
      "content_text_required": true,
      "source_uri_allowed": false
    },
    {
      "kind": "media_transcript",
      "allowed_delivery_modes": ["heard_reported", "viewed"],
      "content_text_required": true,
      "source_uri_allowed": false
    }
  ]
}
```

Return no more than two requirement objects. Use these requirements:

- picture narration: alternatives `image_description` and
  `image_sequence_description`, each delivered as `viewed` with text required;
- retell/reconstruction and dictogloss: alternatives `source_text` and
  `media_transcript`, with delivery and text requirements from section 7.4;
- read-close-explain: alternatives `source_text` and `article_reference`, each
  delivered as `read`; text is required for `source_text` and optional for
  `article_reference`. Every alternative still requires at least one of text or
  URI, matching the schema and section 7.4 matrix;
- role-play with complications: one `situation` alternative, delivered as
  `read` or `conversation`, with text required;
- all other activities: an empty array.

These objects tell ChatGPT what it must generate or obtain before recording.
They do not claim that the MCP server verified real-world delivery.

Return one allocation per planned item. Item numbers must be contiguous from 1.
If target selection returns no active target after applying request filters,
return a domain error and do not return a zero-item activity plan. Allocate
selected targets as evenly as possible: compute each target's allocation with
the current allocator, then flatten occurrences round-robin in selected-target
order while respecting those allocation counts. This defines item numbering and
makes the first occurrence used by `recognition_to_production` unambiguous.

For every mix, if an explicit weakness list contains more targets than `count`,
return a validation error instead of truncating that list. If any selected
target has no eligible recommendation after all constraints, return an error
that names that weakness and the incompatible constraints; do not silently
drop the target and redistribute its items.

In version 8, assign exactly one deliberately targeted weakness to each planned
item. Record other weaknesses noticed in the response as incidental
observations. Do not add multi-target planning in this release.

Assign item phases deterministically in activity briefs:

- corrective conversation: item 1 is `initial`; every later item is
  `follow_up`;
- role-play with complications: item 1 is `initial`; the final
  `complication_count` items are `complication`; intervening items are
  `follow_up`;
- every other activity: all items are `initial`.

For role-play, require the effective complication count to be less than the
effective item count. The default is one. A caller may explicitly override it
to zero, producing an initial/follow-up role-play without a complication even
though complications remain the activity's normal default.

Set `intended_evidence_strength` from the requested activity, not merely from
the chosen drill key:

- `controlled_production`: sentence-transformation sprint and dictogloss;
- `spontaneous_production`: the other eight activities.

For a brief without an activity, derive intended evidence strength from the
selected recommendation stage: recognition maps to `recognition`, controlled
to `controlled_production`, and transfer or fluency to
`spontaneous_production`. This value is planning guidance only; recording an
FSRS review still requires observations satisfying the existing evidence rules.

### 10.5 Activity defaults

Use these defaults only for brief generation. Callers override individual
config fields with `PracticeBriefRequest.activity_config` and may separately
override whole-run duration with `planned_duration_seconds`, within section 9
bounds:

- Situational response: 3 seconds preparation, 45 seconds response.
- Picture narration: 30 seconds preparation, 120 seconds response.
- Q&A sprint: 3 seconds preparation per question, 30 seconds response.
- Retell/reconstruction: one source exposure, 120 seconds response, source
  hidden before response.
- Corrective conversation: six learner turns, 1,200 seconds planned run
  duration, and unpredictable follow-ups; collect corrections during the run
  and summarize recurring errors afterward.
- Sentence transformation: six items, no response-duration inference.
- Dictogloss: two reported exposures, source hidden, 50-100 word source
  requested, 180 seconds reconstruction.
- Voice diary: 180 seconds target response; ask learner afterward for two or
  three self-reported reflections.
- Read-close-explain: source hidden, 120 seconds response.
- Role-play: four learner turns, unpredictable follow-ups, and one complication
  by default.

The `generation_rules` output must describe these as instructions to ChatGPT,
not as facts that the server verified.

When `activity_type` is absent, omit `activity_plan` rather than returning an
empty or null plan, preserving the existing brief shape for old callers. When
it is present, include `planned_duration_seconds` in `activity_plan` when known.

## 11. `record_practice_session` behavior

Within the existing transaction:

1. Validate the complete request and idempotency hash as today.
2. Resolve and insert the session.
3. Insert activity runs and build a `run_no -> activity_run_id` map.
4. Insert each run's stimuli.
5. Insert practice items with the mapped activity run ID and item phase.
6. Insert attempts with response/timing metadata.
7. Insert attempt reflections.
8. Insert observations and reviews using the existing evidence and FSRS rules.
9. Commit the idempotency response with the rest of the transaction.

Extend the response with:

- `activity_run_count`;
- `stimulus_count`;
- `reflection_count`.

The request hash must include all new fields through the existing canonical JSON
hashing. Replaying the identical expanded request must return the same session.
Reusing its idempotency key with changed activity data must fail.

Do not let reflections create observations or ratings implicitly.

## 12. Read APIs

### 12.1 `get_recent_practice`

Add request filters:

- `activity_types: Option<Vec<ActivityType>>`
- `response_modes: Option<Vec<ResponseMode>>`

These are session-selection filters, consistent with the existing drill and
weakness filters. A session matches `activity_types` when at least one of its
runs has a requested type, and matches `response_modes` when at least one of its
attempts has a requested mode. When both filters are present, the session must
satisfy both predicates, but the matching run and attempt need not be related.
After selecting sessions, return all requested children for each selected
session; do not silently prune nonmatching runs or attempts from full output.
An explicitly supplied empty filter list is a validation error rather than an
unbounded match.

When detailed items or attempts are requested, return:

- activity runs with type, declared configuration, duration, and timing source;
- stimulus kind, delivery mode, fingerprint, and text/reference fields;
- each item's run number and phase;
- attempt response mode, reported/measured timing, timing source, and
  reflections.

Return `activity_runs` as a session-level array ordered by `run_no`, with each
run's stimuli nested and ordered by `stimulus_no`. Keep practice items and
attempts in their existing session-level arrays to preserve response
compatibility; identify an item's run with `activity_run_no` rather than
duplicating items below runs. Nest reflections only beneath their attempt and
order them by `reflection_no`.

Keep summary mode concise. In summary mode, add only:

- `activity_types` used in each session;
- run/item/attempt counts;
- total measured duration only when all included durations have known timing
  sources; otherwise return `null`, not an estimate.

Compute summary duration per run to avoid double counting:

1. If a run has `actual_duration_milliseconds` and a timing source, use that run
   duration and ignore its attempt durations for the total.
2. Otherwise, use the sum of its linked attempt durations only when the run has
   at least one linked attempt and every linked attempt has an actual duration
   and timing source.
3. Otherwise, that run's duration is unknown.
4. The session `total_measured_duration_milliseconds` is the sum of the run
   values only when every activity run has a known duration. Return `null` for a
   legacy session with no activity runs or when any run is unknown.

One attempt `timing_source` applies to both
`actual_duration_milliseconds` and `response_latency_milliseconds` when both are
present. If those measurements came from different sources, the caller must
omit one rather than misrepresent provenance; supporting per-measurement source
columns is a follow-up, not part of version 8.

Apply bounds before loading child rows. Do not dereference source URIs.
Return summary `activity_types` as distinct keys sorted ascending so output is
deterministic.

### 12.2 `get_learning_context`

Add a compact `activity_coverage` array for the recent-session window. Each
entry has this exact shape:

```json
{
  "activity_type": "question_answer_sprint",
  "run_count": 3,
  "last_practiced_date": "2026-08-30",
  "attempt_count": 18
}
```

For each activity type:

- count attempts linked through practice items to runs of that type;
- exclude orphan legacy attempts, which have no activity type;
- order entries by `last_practiced_date` descending and then activity key
  ascending;
- apply the existing bounded recent-session window before aggregation.

Do not produce speech-rate, pause, or pronunciation statistics.

### 12.3 `get_data_status`

Read the reported schema version from validated `schema_meta` rather than adding
another hard-coded `8` in the database adapter. Add counts for:

- `activity_runs`;
- `activity_stimuli`;
- `attempt_reflections`.

### 12.4 No new MCP action

Do not add `get_activity_catalog` in this release. The `ActivityType` JSON schema,
brief response, and server instructions provide enough discoverability. Keeping
the tool count unchanged reduces connector churn.

## 13. Server instructions

Update `LearningServer::get_info()` instructions to say, concisely:

- start with `get_practice_brief` for a planned activity;
- ChatGPT generates and presents exercise language;
- record each learner-facing prompt/turn separately;
- spoken responses are stored as transcripts only;
- never infer audio properties or timing from transcript text;
- timing and hesitation data must be explicitly learner-reported or externally
  measured;
- record deliberate FSRS reviews under the existing evidence rules.

Keep the instruction that the user is never asked for database/schema details.

## 14. Migration safety

Implement migration 8 using the existing migration framework, not an ad hoc
startup query.

At the start of the migration transaction, capture counts for every existing
table. For each table that must be rebuilt, also capture in Rust an ordered
projection of every primary key
and pre-existing column value (or a deterministic SHA-256 digest of that
length-prefixed projection). Do not compare concatenated SQL text without
length framing, because different values can otherwise produce the same byte
sequence. The migration-8 transaction must perform these pre-commit checks
after all DDL and data copies, but before inserting registry version 8 or
changing `schema_meta`:

- every table that existed before migration has its captured row count before
  the version-8 registry row is inserted;
- every rebuilt table's final ordered projection or digest equals its captured
  pre-migration value for all pre-existing columns;
- all legacy item links and phases have the expected defaults;
- all ten activity seeds exist and are active;
- `PRAGMA integrity_check` returns `ok`;
- `PRAGMA foreign_key_check` returns no rows.

Then insert migration registry version 8, update `schema_meta` to 8, and commit
them together. The existing external postflight must repeat integrity,
foreign-key, registry, and schema-version checks after commit. If a pre-commit
check fails, the transaction rolls back and the database remains wholly at
schema 7.

Extend `postflight_at` so it verifies both `max(schema_migrations.version)` and
the integer value stored at `schema_meta['schema_version']` equal the expected
version. Its current registry-only check is insufficient. Because the migration
runner invokes postflight after each step, migrations 4 through 8 must update
`schema_meta` to their own version in the same transaction as their registry
row. This change does not rerun already registered migrations; migration 8
brings an existing registry-version-7 database directly to metadata version 8.
Fresh initialization must likewise write version 8 rather than the currently
hard-coded version 7.

After migration:

- all pre-existing application-table row counts must be unchanged;
- `schema_migrations` must contain exactly one additional row, for version 8,
  while `schema_meta` retains its existing row count;
- all pre-existing IDs and text fields must be unchanged;
- legacy `practice_items.activity_run_id` values must all be `NULL`;
- legacy item phases must all be `initial`;
- all ten activity types must exist and be active;
- `PRAGMA integrity_check` must return `ok`;
- `PRAGMA foreign_key_check` must return no rows;
- all migration registry versions 3 through 8 must be present exactly once,
  with the expected names and checksums;
- schema version must be 8.

Test migration on a temporary copy of `data/aprendiendo-live.sqlite3`. Never
modify that source snapshot in a test. Also test initialization from an empty
temporary database.

Do not backfill inferred activity runs from session labels such as `4-3-2`,
`production drill`, or `guided conversation drill`. Historical labels are not
precise enough to infer the structure required by this plan.

## 15. Test plan

### 15.1 Model/schema tests

- Every new enum serializes to the exact snake-case database value.
- `ActivitySpec` covers every enum variant exactly once and matches all ten
  database seed rows.
- Unknown request fields remain rejected.
- Old valid recording payloads still deserialize.
- Serializing an old recording payload after version-8 deserialization produces
  the same canonical JSON and request hash as version 7; add a fixture with
  items and attempts so every new nested optional field is covered.
- New empty database has schema version 8 and all activity seeds.
- Exact schema-3 snapshot migrates through version 8 without data loss.
- Existing schema-7 database migrates to version 8.
- Fresh and migrated databases have equivalent columns, foreign keys, indexes,
  checks, and triggers; do not require byte-identical `sqlite_master.sql` text.
- A forced migration-8 pre-commit failure leaves both the registry and
  `schema_meta` at version 7.
- Foreign-key and integrity checks pass.

### 15.2 Practice-brief unit tests

Add a table-driven test for all ten activity types. Assert:

- returned activity type and interaction mode;
- compatible drill types only;
- correct default count;
- single-response count enforcement;
- expected stimulus requirements;
- expected deterministic item phases;
- intended evidence strength;
- transcript/audio warning in recording rules where relevant.

Add separate tests proving:

- `allowed_drill_types` changes allocations;
- an incompatible allowed list fails;
- every `DrillMix` changes or constrains the plan as specified;
- custom mix without allowed types fails;
- recognition-to-production has ordered recognition and production items;
- activity requests reject translation-only, recognition-to-production, and
  fluency mixes before database target selection;
- recognition-to-production rejects too many explicit targets for its count;
- zero selected targets returns a domain error rather than a zero-item plan;
- recent prompts remain included and bounded;
- output ordering is deterministic.

### 15.3 Recording tests

- Record a six-item Q&A sprint with six transcript attempts.
- Record picture narration with an image-sequence text description.
- Record dictogloss with `heard_reported`, two reported exposures, and no audio.
- Record a voice diary with `spoken_transcript`, externally timed duration, and
  learner reflections.
- Record role-play with initial, follow-up, and complication phases.
- Reject an unknown activity run reference.
- Reject an orphan attempt in an activity-aware session.
- Reject incompatible item drill/activity combinations.
- Reject duplicate run, stimulus, and reflection numbers.
- Reject measured timing without a timing source.
- Accept absent timing.
- Reject dictogloss/read-close-explain without source-hidden declaration.
- Reject picture narration without a visual description.
- Reject an image-description kind with URI-only content, a media transcript
  without text, and every invalid stimulus kind/delivery-mode pairing.
- Reject nonblank-required fields containing only whitespace.
- Reject an assistant-sourced reported hesitation or other reported reflection.
- Reject config fields that do not apply to the declared activity.
- Reject Q&A and role-play config counts that disagree with their linked item
  counts or complication phases.
- Verify reflections do not create observations or reviews.
- Verify all inserted activity data rolls back if review validation fails.
- Verify identical idempotent replay and changed-payload rejection.
- Verify a replayable request recorded before migration still replays after
  migration with its original idempotency key and hash.

### 15.4 Read API tests

- Filter recent practice by activity type.
- Return full run/stimulus/item/attempt/reflection structure in full mode.
- Keep summary mode bounded and omit large stimulus text.
- Verify activity and response-mode filters select sessions but do not prune
  their nonmatching child rows in full mode.
- Verify run duration takes precedence over attempt durations and is never
  double-counted.
- Verify incomplete or unsourced run/attempt timing makes the session summary
  duration `null`.
- Report activity coverage in learning context.
- Never return invented timing or audio metrics.

### 15.5 Protocol tests

- Tool count remains unchanged.
- Updated JSON schemas expose activity enums and nested recording fields.
- Representative old and new MCP calls succeed.
- Oversized text, reflections, and full payloads fail with clear
  validation errors.
- Tool annotations remain correct: reads are read-only and recording is
  non-destructive but mutating.

## 16. Implementation sequence

Complete and verify each phase before starting the next.

### Phase 1: Make drill planning truthful

1. Introduce the typed recommendation record and extract deterministic drill
   eligibility/allocation helpers from `practice_brief`.
2. Implement every existing `DrillMix` rule.
3. Enforce `allowed_drill_types`.
4. Add unit tests for filtering, allocation, and incompatible requests.

Do not proceed while the API still accepts constraints it ignores.

### Phase 2: Add activity models and the catalog

1. Add all enums and input structures from section 8.
2. Add the exhaustive `ActivitySpec` catalog and its coverage tests.
3. Add pure validation helpers for config applicability, stimulus
   compatibility, and drill/activity compatibility.

### Phase 3: Add schema version 8

1. Add the migration registry entry, atomic schema-version update, and shared
   seed helper backed by `ActivitySpec`.
2. Add activity tables and attempt columns.
3. Rebuild `practice_items` and every dependent table named in section 7.3,
   preserving IDs and existing values.
4. Update fresh-schema SQL.
5. Add empty-database, schema-7, and exact-snapshot migration tests.
6. Run integrity and foreign-key checks.

### Phase 4: Add the write path

1. Add bounded server validation.
2. Repeat relational validation in the database adapter.
3. Insert runs, stimuli, item links/phases, attempt metadata, and reflections in
   the existing transaction.
4. Extend idempotent response counts.
5. Add atomicity and replay tests.

### Phase 5: Add activity-aware briefs

1. Add optional `activity_type`, `planned_duration_seconds`, and
   `activity_config` inputs.
2. Add typed activity-config overrides, count semantics, and activity defaults.
3. Produce explicit per-item allocations and stimulus requirements.
4. Add all ten table-driven activity tests.

### Phase 6: Add read/reporting support

1. Add recent-practice filters and detailed nested output.
2. Add compact activity coverage to learning context.
3. Extend data-status counts/version.
4. Update server instructions and README tool behavior.
5. Add protocol and response-bound tests.

### Phase 7: Verification

Run at least:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

Run the migration command only against temporary database copies during normal
development. Do not release unless explicitly requested.

## 17. Acceptance criteria

The work is complete only when all of the following are true:

1. All ten activities can be requested through `get_practice_brief` and return
   an explicit, compatible, deterministic plan.
2. `drill_mix` and `allowed_drill_types` materially constrain the returned
   allocations and cannot be silently ignored.
3. A session can atomically record activity runs, text/reference stimuli,
   per-item turns, transcripts, explicitly sourced timing, reflections,
   observations, and reviews.
4. No schema, API response, instruction, or test claims access to actual audio.
5. No timing, pause, pronunciation, or acoustic-fluency metric is inferred from
   transcript text.
6. Existing recording payloads remain valid.
7. Existing schema-3 data reaches schema 8 without inferred activity backfill or
   loss of historical data.
8. Read APIs can filter and summarize activity usage without unbounded output.
9. FSRS behavior and the `general` scheduler track remain unchanged except for
   receiving properly validated evidence from the new activities.
10. Formatting, lint, tests, release build, integrity checks, and foreign-key
    checks all pass.

## 18. Explicit follow-ups not included in version 8

After enough deliberate activity data has accumulated, separately evaluate:

- whether spontaneous-production FSRS state should be split from the general
  track;
- whether a client capable of trustworthy timestamp capture should populate
  response latency automatically;
- whether stable reflection patterns should become candidate weaknesses;
- whether activity adherence/cadence needs a non-FSRS scheduler;
- whether activity catalog metadata warrants a dedicated read action.

Do not implement these follow-ups as incidental additions to this plan.
