# Taxonomy, Evidence, and Data Integrity Implementation Plan

## 1. Purpose and authority

This document specifies the next data-model revision for Aprendiendo. It is
written for an implementation agent that has no context beyond this repository
and this file. Decisions marked **Decision** are settled. Do not replace them
with a different taxonomy, a graph database, RDF storage, inferred historical
FSRS reviews, or a destructive rewrite of existing data.

This plan extends the FSRS work described in `FSRS_PLAN.md`. Where the two plans
conflict, this document controls the taxonomy, evidence, historical-bootstrap,
parameter-history, and schema-migration behavior. In particular, do not run the
existing automatic `reconstruct_legacy_reviews` behavior against the production
snapshot.

The implementation must:

1. preserve every existing session, attempt, observation, weakness, and text
   field;
2. keep all 39 current weakness IDs and keys stable;
3. add an extensible, typed concept graph around those weaknesses;
4. keep FSRS attached only to assessable learner targets, never to taxonomy
   parents, concepts, categories, or collections;
5. make future weaknesses recordable even before their final classification is
   known;
6. distinguish cold retrieval evidence from teaching, correction, immediate
   retry, transfer, and incidental evidence;
7. keep enough immutable history to reproduce scheduler state and later train
   personalized FSRS parameters;
8. repair the known migration, timestamp, idempotency, query-output, and
   relational-integrity issues described below.

No production release is part of implementation unless explicitly requested.
When a release is requested, the repository instruction is to run
`scripts/release.sh`.

## 2. Exact migration source

Treat `data/aprendiendo-live.sqlite3` as the exact pre-migration production
shape and contents. Before changing a copy of it, assert all of the following.
Abort the migration if an assertion fails; do not attempt a best-effort repair.

### 2.1 Row counts and dates

| Table | Rows before migration |
| --- | ---: |
| `sessions` | 17 |
| `attempts` | 24 |
| `observations` | 162 |
| `weaknesses` | 39 |
| `practice_items` | 0 |
| `practice_item_targets` | 0 |
| `weakness_reviews` | 0 |
| `scheduler_config` | 1 |
| `recorded_requests` | 2 |
| `schema_meta` | 1 |

There are 11 distinct learning/session dates from `2026-08-02` through
`2026-08-28`. All 17 sessions have observations. SQLite `integrity_check` is
`ok`, `foreign_key_check` returns no rows, and no attempt-linked observation
points to an attempt from another session.

### 2.2 Existing evidence

The 162 observations comprise:

- 82 `incorrect` and 80 `correct` outcomes;
- 131 attempt-linked observations and 31 session-level observations;
- zero practice-item-linked observations;
- 162 `role='incidental'` observations;
- zero non-null `evidence_strength` values;
- zero non-null `severity` values;
- 162 non-null produced responses;
- 142 non-null corrections and 153 non-null notes.

They form 108 distinct `(session_id, weakness_id)` groups. Of those groups, 44
are entirely incorrect, 41 are entirely correct, and 23 contain both outcomes.
Seventy-one groups contain only one observation. These facts are why the
history must not be converted automatically into deliberate FSRS reviews.

There are five `4-3-2` sessions. No existing attempt has a
`target_duration_seconds` value. Do not infer timing, words per minute, pause
counts, fluency, hint usage, retrieval mode, or assessment phase from a
transcript.

### 2.3 Existing weakness and scheduler state

All 39 weaknesses are active, have non-empty descriptions and target patterns,
have at least one observation, have no explicit drill recommendations, and have
null FSRS stability, difficulty, due, last-review, algorithm-version, and
parameter-version fields. `weakness_reviews` is empty.

The legacy cache has due dates from `2026-08-16` through `2026-08-29`, intervals
of only 0 or 1 day, repetitions of only 0 or 1, and lapses from 0 through 2. It
is not reliable enough to initialize FSRS with `memory_state_from_sm2`.

The singleton scheduler configuration is:

- algorithm `fsrs`;
- algorithm version `FSRS-6`;
- desired retention approximately `0.90`;
- parameter version 1;
- source `default`;
- timezone `Europe/Madrid`;
- day cutoff 4;
- a 21-element parameter vector.

### 2.4 Known historical anomalies to preserve or explicitly mark

- The two `recorded_requests` rows have empty request hashes and `{}` responses.
  They are not replayable.
- Timestamp formats are mixed. There are 15 legacy and 2 canonical session
  timestamps, 22 legacy and 2 canonical attempt timestamps, and 135 legacy and
  27 canonical observation timestamps. All 39 weakness timestamps and both
  recorded-request timestamps are already canonical.
- One exact duplicate observation signature exists. Preserve both rows; do not
  guess which one is authoritative.
- `adverb_invariable_modifier.first_seen` is `2026-08-21`, while its first
  linked session is `2026-08-22`. Preserve `first_seen`; it can represent
  discovery/catalog time rather than first stored evidence.
- `schema_meta` contains `schema_version=3`.

## 3. Terminology and domain boundaries

Use these terms consistently in SQL, Rust, documentation, and API output.

### 3.1 Concept

A reusable linguistic, communicative, or contextual idea such as agreement,
subjunctive mood, temporal sequence, politeness mitigation, or health. Concepts
organize and relate targets. Concepts do not have learner observations or FSRS
state.

### 3.2 Concept scheme

An independent classification facet. The initial schemes are linguistic form,
communicative use, and meaning/context. A target may link to concepts in several
schemes.

### 3.3 Learning target

An assessable behavior with a stable identity and a coherent rating history,
for example “use `para que` plus subjunctive when subjects differ.” The current
`weaknesses` rows are learning targets. Retain the table name during this
revision to avoid an unnecessary, high-risk rename.

### 3.4 Learner-target status

Whether a target is merely a candidate discovered incidentally, actively being
trained, suspended, retired, or replaced by more precise targets. This is not a
taxonomy relation and is not equivalent to FSRS due state.

### 3.5 Scheduler item

The exact memory unit scheduled by FSRS. Initially there is one `general` track
per active current weakness. The separate table allows a future split into
controlled and spontaneous production without changing the weakness or concept
identity.

### 3.6 Collection

A curated, possibly ordered group for a changing study purpose such as DELE A2,
travel, health care, or a monthly remediation campaign. A collection is not a
parent concept and is never scheduled.

### 3.7 Observation and review

An observation is raw evidence about one weakness in one learner response. A
review is the single explicit FSRS rating derived from selected deliberate
evidence for one scheduler item in one session. Multiple observations may
support one review. Immediate retries are observations, not additional spaced
reviews.

## 4. Settled architecture

**Decision:** Implement a typed, faceted graph in ordinary SQLite tables. Do not
add Neo4j, another graph service, RDF storage, or an ontology runtime.

The model is inspired by SKOS's separation of concept schemes, hierarchical
relations, associative relations, and collections, but it uses application-
specific relational tables and predicates. Reference:
`https://www.w3.org/TR/skos-reference/`.

The high-level relationship is:

```text
concept_schemes -> concepts -> concept_edges
                         ^
                         |
weaknesses -> weakness_concepts
     |
     +-> weakness_relations
     +-> observations
     +-> scheduler_items -> weakness_reviews -> review_observations
     +-> collection_members -> collections
```

**Decision:** A concept hierarchy may be a directed acyclic graph. A concept may
have more than one immediate `broader` concept when that is genuinely useful.
The application must reject cycles.

**Decision:** Do not use one generic `parent_key`. The meanings of `broader`,
`requires`, `contrasts_with`, `confusable_with`, `supersedes`, and collection
membership are different and must stay distinguishable.

**Decision:** Every non-retired weakness has exactly one primary concept for
default navigation. It may have any number of additional form, meaning,
function, context, error-source, or curriculum links.

**Decision:** The existing `weaknesses.category` is legacy compatibility data.
Keep it populated and returned during the transition, but stop using it as the
source of hierarchy, target diversification, or new classification.

## 5. Taxonomy schema

Add the following tables to `sql/sqlite_schema.sql` and to an explicit versioned
migration. Use the same column definitions in both paths.

### 5.1 Schemes and concepts

```sql
CREATE TABLE concept_schemes (
  id INTEGER PRIMARY KEY,
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  description TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE concepts (
  id INTEGER PRIMARY KEY,
  scheme_id INTEGER NOT NULL REFERENCES concept_schemes(id),
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  definition TEXT,
  source_uri TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX concepts_scheme_idx
  ON concepts(scheme_id, active, sort_order, key);
```

Concept keys are immutable stable identifiers. Labels and definitions may be
edited. A key must be lower-case ASCII segments separated by dots. Validate it
in Rust with the equivalent of `^[a-z0-9]+(?:[._-][a-z0-9]+)*$` and a maximum
length of 160 characters.

### 5.2 Concept edges

```sql
CREATE TABLE concept_edges (
  subject_id INTEGER NOT NULL REFERENCES concepts(id),
  predicate TEXT NOT NULL CHECK (predicate IN (
    'broader', 'requires', 'contrasts_with', 'related'
  )),
  object_id INTEGER NOT NULL REFERENCES concepts(id),
  provenance TEXT NOT NULL DEFAULT 'curated' CHECK (provenance IN (
    'curated', 'external', 'inferred'
  )),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (subject_id, predicate, object_id),
  CHECK (subject_id <> object_id)
);

CREATE INDEX concept_edges_object_idx
  ON concept_edges(object_id, predicate, subject_id);
```

Predicate meanings are strict:

- `subject broader object`: `object` is the immediate broader/parent concept
  and `subject` is narrower. Store only immediate links; use recursive CTEs for
  ancestors and descendants. This follows the conventional SKOS direction.
- `subject requires object`: competence in `object` is a reasonable prerequisite
  for learning or producing `subject`.
- `contrasts_with`: a symmetric semantic contrast suitable for discrimination
  practice.
- `related`: an associative relation that is neither hierarchical nor a known
  prerequisite. Use it sparingly.

For symmetric predicates, store a canonical pair ordered by concept key. Query
helpers must search both columns. For `broader` and `requires`, direction is
meaningful. Before inserting either directed predicate, run a recursive query
and reject a cycle. Add unit tests for direct and indirect cycles.

### 5.3 Weakness classification and lifecycle

Add to `weaknesses`:

```sql
target_type TEXT NOT NULL DEFAULT 'grammatical_construction'
  CHECK (target_type IN (
    'grammatical_construction',
    'lexical_item',
    'lexical_chunk',
    'form_meaning_contrast',
    'pronunciation',
    'orthography',
    'discourse_strategy',
    'sociopragmatic_choice'
  )),
target_status TEXT NOT NULL DEFAULT 'active'
  CHECK (target_status IN (
    'candidate', 'active', 'suspended', 'retired', 'merged'
  )),
classification_pending INTEGER NOT NULL DEFAULT 0
  CHECK (classification_pending IN (0,1))
```

For the current 39 rows, set `target_status='active'` and
`classification_pending=0`. Continue mirroring the legacy `active` boolean:
`active=1` for candidate/active and `active=0` for suspended/retired/merged.
Centralize this mapping in one Rust helper and test it. New queries should use
`target_status`, not the boolean.

```sql
CREATE TABLE weakness_concepts (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  concept_id INTEGER NOT NULL REFERENCES concepts(id),
  role TEXT NOT NULL CHECK (role IN (
    'primary', 'form', 'meaning', 'function', 'context',
    'error_source', 'curriculum'
  )),
  provenance TEXT NOT NULL DEFAULT 'curated' CHECK (provenance IN (
    'curated', 'external', 'inferred'
  )),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (weakness_id, concept_id, role)
);

CREATE UNIQUE INDEX weakness_one_primary_concept_idx
  ON weakness_concepts(weakness_id)
  WHERE role='primary';

CREATE INDEX weakness_concepts_concept_idx
  ON weakness_concepts(concept_id, role, weakness_id);
```

Rust validation must require one primary link for every candidate or active
target before transaction commit. SQLite cannot express “exactly one related
row” with a simple check constraint.

### 5.4 Relations between assessable targets

```sql
CREATE TABLE weakness_relations (
  subject_id INTEGER NOT NULL REFERENCES weaknesses(id),
  predicate TEXT NOT NULL CHECK (predicate IN (
    'confusable_with', 'variant_of', 'supersedes', 'practice_together'
  )),
  object_id INTEGER NOT NULL REFERENCES weaknesses(id),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (subject_id, predicate, object_id),
  CHECK (subject_id <> object_id)
);

CREATE INDEX weakness_relations_object_idx
  ON weakness_relations(object_id, predicate, subject_id);
```

`confusable_with`, `variant_of`, and `practice_together` are symmetric; store a
canonical key-ordered pair and query both directions. `supersedes` is directed:
the subject is the new target and the object is the old target.

Do not move FSRS state through `supersedes`. When a broad target is split, retain
and retire the old scheduler item, create new uninitialized scheduler items,
and keep all history linked to the old target. A pure label or key-alias change
must preserve the original target ID and scheduler item.

### 5.5 Collections

```sql
CREATE TABLE collections (
  id INTEGER PRIMARY KEY,
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  collection_type TEXT NOT NULL CHECK (collection_type IN (
    'goal', 'curriculum', 'topic', 'campaign'
  )),
  description TEXT,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE collection_members (
  collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  position INTEGER,
  notes TEXT,
  PRIMARY KEY (collection_id, weakness_id)
);
```

Do not store `due`, `new`, `recently_lapsed`, or similar changing queue states as
collections. Those are queries over scheduler and review data.

### 5.6 External curriculum mappings

Do not copy the complete PCIC, CEFR, RAE, or Universal Dependencies inventories
into this database. Store selective references needed by current targets.

```sql
CREATE TABLE curriculum_mappings (
  id INTEGER PRIMARY KEY,
  weakness_id INTEGER REFERENCES weaknesses(id),
  concept_id INTEGER REFERENCES concepts(id),
  framework TEXT NOT NULL CHECK (framework IN ('PCIC','CEFR','UD','RAE')),
  external_key TEXT NOT NULL,
  external_uri TEXT,
  relation TEXT NOT NULL CHECK (relation IN (
    'exact', 'close', 'broader', 'narrower', 'related'
  )),
  level_min TEXT,
  level_max TEXT,
  notes TEXT,
  CHECK ((weakness_id IS NULL) <> (concept_id IS NULL))
);

CREATE UNIQUE INDEX curriculum_mapping_weakness_idx
  ON curriculum_mappings(weakness_id, framework, external_key)
  WHERE weakness_id IS NOT NULL;

CREATE UNIQUE INDEX curriculum_mapping_concept_idx
  ON curriculum_mappings(concept_id, framework, external_key)
  WHERE concept_id IS NOT NULL;
```

CEFR level is a mapping or range, never a taxonomy parent. UD features such as
`Mood=Sub`, `Tense=Imp`, `Gender`, and `Number` are optional machine-readable
annotations, not the learner-facing organization.

## 6. Initial taxonomy seed

Seed the following schemes with stable keys:

| Key | Label |
| --- | --- |
| `linguistic_form` | Linguistic form |
| `communicative_use` | Communicative use |
| `meaning_context` | Meaning and context |

### 6.1 Concept seed

In the following table, `Broader` is a concept key or blank for a top concept.
Insert one `concept_edges(..., 'broader', ...)` row for every non-blank value,
using the direction defined in section 5.2.

| Concept key | Scheme | Label | Broader |
| --- | --- | --- | --- |
| `form.unclassified` | linguistic_form | Unclassified linguistic form | |
| `form.grammar` | linguistic_form | Grammar | |
| `form.grammar.morphology` | linguistic_form | Morphology | `form.grammar` |
| `form.grammar.morphology.agreement` | linguistic_form | Agreement | `form.grammar.morphology` |
| `form.grammar.morphology.invariability` | linguistic_form | Invariable modifiers | `form.grammar.morphology` |
| `form.grammar.verb_system` | linguistic_form | Verb system | `form.grammar` |
| `form.grammar.verb_system.tense_aspect` | linguistic_form | Tense and aspect | `form.grammar.verb_system` |
| `form.grammar.verb_system.mood` | linguistic_form | Mood and modality | `form.grammar.verb_system` |
| `form.grammar.verb_system.mood.subjunctive` | linguistic_form | Subjunctive mood | `form.grammar.verb_system.mood` |
| `form.grammar.verb_system.periphrasis` | linguistic_form | Verbal periphrases | `form.grammar.verb_system` |
| `form.grammar.syntax` | linguistic_form | Syntax | `form.grammar` |
| `form.grammar.syntax.clitics` | linguistic_form | Object and reflexive clitics | `form.grammar.syntax` |
| `form.grammar.syntax.valency` | linguistic_form | Valency and argument structure | `form.grammar.syntax` |
| `form.grammar.syntax.subordination` | linguistic_form | Subordination | `form.grammar.syntax` |
| `form.grammar.syntax.prepositions` | linguistic_form | Prepositional selection | `form.grammar.syntax` |
| `form.grammar.syntax.copular` | linguistic_form | Copular constructions | `form.grammar.syntax` |
| `form.lexis` | linguistic_form | Lexis and phraseology | |
| `form.lexis.lexeme_choice` | linguistic_form | Lexeme choice | `form.lexis` |
| `form.lexis.collocation` | linguistic_form | Collocations | `form.lexis` |
| `form.lexis.fixed_expression` | linguistic_form | Fixed expressions | `form.lexis` |
| `form.lexis.lexical_grammar` | linguistic_form | Lexically governed grammar | `form.lexis` |
| `form.pronunciation` | linguistic_form | Pronunciation and prosody | |
| `form.orthography` | linguistic_form | Orthography | |
| `use.unclassified` | communicative_use | Unclassified communicative use | |
| `use.describe_narrate_past` | communicative_use | Describe and narrate in the past | |
| `use.evaluate_completed_event` | communicative_use | Evaluate a completed event | |
| `use.report_influence` | communicative_use | Report commands and influence | |
| `use.express_purpose` | communicative_use | Express purpose | |
| `use.express_duration` | communicative_use | Express duration | |
| `use.express_cause` | communicative_use | Express cause | |
| `use.express_space` | communicative_use | Express spatial relations | |
| `use.express_desire_politeness` | communicative_use | Express desire and mitigation | |
| `use.social_interaction` | communicative_use | Manage social interaction | |
| `use.discourse` | communicative_use | Organize discourse | |
| `use.sociopragmatics` | communicative_use | Register and sociopragmatic choice | |
| `meaning.unclassified` | meaning_context | Unclassified meaning or context | |
| `meaning.time` | meaning_context | Time and temporal sequence | |
| `meaning.duration` | meaning_context | Duration | |
| `meaning.cause` | meaning_context | Cause | |
| `meaning.purpose` | meaning_context | Purpose | |
| `meaning.space` | meaning_context | Space and location | |
| `meaning.emotion_reaction` | meaning_context | Emotion and reaction | |
| `domain.health` | meaning_context | Health and the body | |
| `domain.travel` | meaning_context | Travel and transport | |
| `domain.books_reading` | meaning_context | Books and reading | |
| `domain.social_relations` | meaning_context | Social relationships | |

Add these initial non-hierarchical concept edges:

- `form.grammar.syntax.clitics requires form.grammar.syntax.valency`;
- `form.grammar.syntax.subordination requires form.grammar.verb_system.mood`;
- `form.grammar.verb_system.mood.subjunctive related form.grammar.syntax.subordination`;
- `use.evaluate_completed_event related use.describe_narrate_past`;
- `meaning.duration related meaning.time`.

Do not add speculative prerequisites beyond this small seed.

### 6.2 Exact mapping of the 39 current weaknesses

Every key below already exists and must retain its current ID, description,
target pattern, dates, legacy scheduler cache, and observations. Set the listed
target type, create the primary link, and create all additional links. The
semicolon-separated additional entries use `role:concept_key`.

| Weakness key | Target type | Primary concept | Additional links |
| --- | --- | --- | --- |
| `adverb_invariable_modifier` | grammatical_construction | `form.grammar.morphology.invariability` | form:`form.grammar.morphology.agreement` |
| `agreement_gender_number` | grammatical_construction | `form.grammar.morphology.agreement` | form:`form.grammar.syntax` |
| `acompanar_direct_object` | grammatical_construction | `form.grammar.syntax.valency` | form:`form.grammar.syntax.prepositions`; context:`domain.social_relations` |
| `doler_gustar_structure` | grammatical_construction | `form.grammar.syntax.valency` | form:`form.grammar.syntax.clitics`; context:`domain.health` |
| `impedir_a_alguien` | grammatical_construction | `form.grammar.syntax.valency` | form:`form.grammar.syntax.prepositions`; form:`form.grammar.syntax.clitics` |
| `duration_llevar_gerund` | grammatical_construction | `form.grammar.verb_system.periphrasis` | function:`use.express_duration`; meaning:`meaning.duration` |
| `iba_a_infinitive` | grammatical_construction | `form.grammar.verb_system.periphrasis` | form:`form.grammar.verb_system.tense_aspect`; meaning:`meaning.time` |
| `aplicarse_clitics` | grammatical_construction | `form.grammar.syntax.clitics` | form:`form.grammar.syntax.valency`; context:`domain.health` |
| `decirselo_form` | grammatical_construction | `form.grammar.syntax.clitics` | form:`form.orthography` |
| `double_object_clitics` | grammatical_construction | `form.grammar.syntax.clitics` | form:`form.grammar.syntax.valency` |
| `hacer_pensar` | grammatical_construction | `form.grammar.syntax.clitics` | form:`form.grammar.syntax.valency` |
| `prometer_infinitive_clitics` | grammatical_construction | `form.grammar.syntax.clitics` | form:`form.grammar.syntax.valency` |
| `aprovechar_para` | lexical_chunk | `form.lexis.lexical_grammar` | form:`form.lexis.collocation`; function:`use.express_purpose`; meaning:`meaning.purpose` |
| `club_de_lectura` | lexical_chunk | `form.lexis.collocation` | context:`domain.books_reading` |
| `cometer_error` | lexical_chunk | `form.lexis.collocation` | |
| `dar_estres_energia` | lexical_chunk | `form.lexis.collocation` | meaning:`meaning.emotion_reaction` |
| `faltar_tiempo` | lexical_chunk | `form.lexis.collocation` | meaning:`meaning.time`; function:`use.express_duration` |
| `hinchazon_disminuir` | lexical_chunk | `form.lexis.collocation` | form:`form.lexis.lexeme_choice`; context:`domain.health` |
| `reservar_asientos` | lexical_chunk | `form.lexis.collocation` | form:`form.lexis.lexeme_choice`; context:`domain.travel` |
| `cuanto_antes` | lexical_chunk | `form.lexis.fixed_expression` | meaning:`meaning.time` |
| `durante_todo_periodo` | lexical_chunk | `form.lexis.fixed_expression` | meaning:`meaning.duration`; function:`use.express_duration` |
| `tratar_de_topic` | grammatical_construction | `form.lexis.lexical_grammar` | form:`form.grammar.syntax.valency`; context:`domain.books_reading` |
| `body_part_prepositions` | grammatical_construction | `form.grammar.syntax.prepositions` | context:`domain.health`; meaning:`meaning.space` |
| `caused_by` | grammatical_construction | `form.grammar.syntax.prepositions` | function:`use.express_cause`; meaning:`meaning.cause` |
| `despedirse_de` | grammatical_construction | `form.grammar.syntax.prepositions` | form:`form.lexis.lexical_grammar`; function:`use.social_interaction`; context:`domain.social_relations` |
| `salir_de_place_event` | grammatical_construction | `form.grammar.syntax.prepositions` | meaning:`meaning.space`; context:`domain.travel` |
| `space_para_piernas` | grammatical_construction | `form.grammar.syntax.prepositions` | function:`use.express_space`; meaning:`meaning.space`; context:`domain.travel` |
| `ser_estar_event_result` | form_meaning_contrast | `form.grammar.syntax.copular` | function:`use.evaluate_completed_event` |
| `antes_de_que_subjunctive` | grammatical_construction | `form.grammar.syntax.subordination` | form:`form.grammar.verb_system.mood.subjunctive`; meaning:`meaning.time` |
| `para_que_subjunctive` | grammatical_construction | `form.grammar.syntax.subordination` | form:`form.grammar.verb_system.mood.subjunctive`; function:`use.express_purpose`; meaning:`meaning.purpose` |
| `past_nonspecific_relative_subjunctive` | grammatical_construction | `form.grammar.syntax.subordination` | form:`form.grammar.verb_system.mood.subjunctive`; form:`form.grammar.verb_system.tense_aspect` |
| `reported_command_past` | grammatical_construction | `form.grammar.syntax.subordination` | form:`form.grammar.verb_system.mood.subjunctive`; function:`use.report_influence` |
| `reported_influence_past` | grammatical_construction | `form.grammar.syntax.subordination` | form:`form.grammar.verb_system.mood.subjunctive`; function:`use.report_influence` |
| `preterite_ayer` | grammatical_construction | `form.grammar.verb_system.tense_aspect` | meaning:`meaning.time`; function:`use.describe_narrate_past` |
| `preterite_completed_evaluation` | grammatical_construction | `form.grammar.verb_system.tense_aspect` | function:`use.evaluate_completed_event`; function:`use.describe_narrate_past` |
| `preterite_imperfect_background` | form_meaning_contrast | `form.grammar.verb_system.tense_aspect` | function:`use.describe_narrate_past` |
| `queria_querria_quisiera` | form_meaning_contrast | `form.grammar.verb_system.mood` | form:`form.grammar.verb_system.tense_aspect`; function:`use.express_desire_politeness`; function:`use.sociopragmatics` |
| `farmaceutico_vocab` | lexical_item | `form.lexis.lexeme_choice` | context:`domain.health` |
| `paraguas_vocab` | lexical_item | `form.lexis.lexeme_choice` | context:`domain.travel` |

Seed these target relations, using canonical key ordering for symmetric rows:

- `reported_command_past practice_together reported_influence_past`;
- `preterite_completed_evaluation confusable_with preterite_imperfect_background`;
- `preterite_ayer practice_together preterite_imperfect_background`;
- `decirselo_form practice_together double_object_clitics`;
- `double_object_clitics practice_together prometer_infinitive_clitics`;
- `aplicarse_clitics practice_together double_object_clitics`.

Do not split `queria_querria_quisiera` or `ser_estar_event_result` during this
migration. Their existing evidence is too sparse. If future evidence shows that
their components have different outcomes or schedules, create new targets,
connect them with directed `supersedes` edges, and retire rather than delete the
old target.

## 7. Future weakness workflow

### 7.1 Granularity rule

A new weakness should be one learning target only when one cold retrieval can
produce one meaningful FSRS rating. Split a proposed target when its components
could plausibly be recalled independently or require different interventions.
Do not create a separate FSRS target for every generated prompt; prompts are
cues for a target.

### 7.2 Classification behavior

Extend `NewWeaknessInput` and `UpsertWeaknessRequest` with:

```rust
target_type: Option<TargetType>,
primary_concept_key: Option<String>,
concept_links: Vec<ConceptLinkInput>,
target_relations: Vec<TargetRelationInput>,
```

`ConceptLinkInput` contains `concept_key` and a non-primary `role`.
`TargetRelationInput` contains `other_weakness_key`, `predicate`, and optional
notes.

If a new weakness has an explicit primary concept, validate that all concepts
exist and write the target plus its classification atomically. If classification
is omitted, select one of the three unclassified concepts using `target_type`,
set `classification_pending=1`, and return that fact in the response. Never
reject raw learning evidence merely because a fine-grained concept has not yet
been curated.

A weakness discovered only through incidental evidence defaults to
`target_status='candidate'` and does not receive an active scheduler item. A
weakness explicitly declared as a practice-item target in that same request may
start as `active`; create its `general` scheduler item atomically. Promoting a
candidate to active creates the uninitialized scheduler item. Suspending,
retiring, or merging a target deactivates but does not delete its scheduler
items. Do not promote or retire targets automatically from outcome counts.

Mapping for unclassified targets:

- pronunciation or orthography use their corresponding form concepts;
- discourse strategy and sociopragmatic choice use `use.unclassified`;
- all other target types use `form.unclassified`.

Extend `upsert_weakness` so a later call can replace the primary concept and all
additional links in one transaction. Do not implicitly create a concept from a
free-text category. Concept creation is a deliberate operation.

### 7.3 Taxonomy maintenance operation

Add a bounded `upsert_concept` MCP tool with these inputs:

- concept key, scheme key, label, definition, optional source URI;
- optional immediate broader keys;
- optional typed edges.

It may create or update one concept and at most 20 edges per request. It must
validate schemes, predicates, symmetric canonicalization, text limits, and DAG
cycles before committing. It must not delete concepts. Deactivation is allowed
only when no candidate or active target uses the concept as primary.

Add a read-only `get_taxonomy` tool supporting:

- `scheme_keys`, `root_concept_keys`, `include_descendants`, and depth limited
  to 1 through 6;
- optional target inclusion;
- optional collection inclusion;
- a hard result limit;
- compact and full detail modes.

## 8. Drill recommendations and prompt organization

The current 39 `recommended_drill_types` values are empty. Stop treating the
JSON column as authoritative. Keep it as legacy compatibility data and replace
it with normalized recommendations inherited from concepts.

```sql
CREATE TABLE concept_drill_recommendations (
  concept_id INTEGER NOT NULL REFERENCES concepts(id),
  drill_type TEXT NOT NULL CHECK (drill_type IN (
    'translation', 'situational_response', 'sentence_transformation',
    'question_answer', 'sentence_completion', 'sentence_combining',
    'error_correction', 'minimal_pair_choice', 'dialogue_completion',
    'micro_story', 'retell', 'fluency_4_3_2'
  )),
  stage TEXT NOT NULL CHECK (stage IN (
    'recognition', 'controlled', 'transfer', 'fluency'
  )),
  weight REAL NOT NULL CHECK (weight > 0 AND weight <= 1),
  PRIMARY KEY (concept_id, drill_type, stage)
);

CREATE TABLE weakness_drill_overrides (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  drill_type TEXT NOT NULL CHECK (drill_type IN (
    'translation', 'situational_response', 'sentence_transformation',
    'question_answer', 'sentence_completion', 'sentence_combining',
    'error_correction', 'minimal_pair_choice', 'dialogue_completion',
    'micro_story', 'retell', 'fluency_4_3_2'
  )),
  stage TEXT NOT NULL CHECK (stage IN (
    'recognition', 'controlled', 'transfer', 'fluency'
  )),
  weight REAL NOT NULL CHECK (weight >= 0 AND weight <= 1),
  PRIMARY KEY (weakness_id, drill_type, stage)
);
```

Seed conservative recommendations:

- agreement, clitics, valency, prepositions, tense/aspect, mood, and
  subordination: `sentence_transformation` and `sentence_completion` for
  controlled production; `translation` as a lower-weight controlled option;
  `situational_response` for transfer;
- form/meaning contrasts: `minimal_pair_choice` for recognition,
  `error_correction` for controlled work, and `situational_response` for
  transfer;
- lexical choice, collocations, fixed expressions, and lexical grammar:
  `sentence_completion` and `translation` for controlled production, followed
  by `dialogue_completion` or `micro_story` for transfer;
- pronunciation/prosody: leave unseeded until supported drill types exist;
- discourse: `retell`, `micro_story`, and `fluency_4_3_2` for transfer/fluency.

Use these exact weights when seeding the applicable broader concepts:

| Concept group | Drill | Stage | Weight |
| --- | --- | --- | ---: |
| grammar concepts listed above | `sentence_transformation` | controlled | 1.00 |
| grammar concepts listed above | `sentence_completion` | controlled | 0.90 |
| grammar concepts listed above | `translation` | controlled | 0.70 |
| grammar concepts listed above | `situational_response` | transfer | 0.80 |
| form/meaning contrast targets via override | `minimal_pair_choice` | recognition | 0.70 |
| form/meaning contrast targets via override | `error_correction` | controlled | 0.90 |
| form/meaning contrast targets via override | `situational_response` | transfer | 1.00 |
| lexis and phraseology concepts | `sentence_completion` | controlled | 0.90 |
| lexis and phraseology concepts | `translation` | controlled | 0.70 |
| lexis and phraseology concepts | `dialogue_completion` | transfer | 0.90 |
| lexis and phraseology concepts | `micro_story` | transfer | 0.70 |
| `use.discourse` | `retell` | transfer | 1.00 |
| `use.discourse` | `micro_story` | transfer | 0.80 |
| `use.discourse` | `fluency_4_3_2` | fluency | 1.00 |

“Grammar concepts listed above” means agreement, invariability, tense/aspect,
mood, periphrasis, clitics, valency, subordination, prepositions, and copular
constructions. “Lexis and phraseology” means lexeme choice, collocation, fixed
expression, and lexical grammar. Add the contrast rows as weakness overrides
for `ser_estar_event_result`, `preterite_imperfect_background`, and
`queria_querria_quisiera`.

Resolution order is target override, then directly linked concept, then nearest
broader concept, then the existing hard-coded fallback. Deduplicate drill types
and keep the highest weight. Never choose a recognition-only session as the
sole evidence for a production target's review unless the caller explicitly
requests recognition.

Add to `practice_items`:

```sql
prompt_fingerprint TEXT,
```

When rebuilding `practice_items`, add the same 12-value `drill_type` check used
above. Keep the Rust `DrillType` enum and these SQL values synchronized with a
test that covers every variant.

For every new item, normalize the prompt by Unicode lowercasing, trimming, and
collapsing whitespace, while retaining accents and punctuation. Store the
lower-case hex SHA-256 digest. Index `(prompt_fingerprint, created_at)` but do
not make it unique. Use it to report exact normalized reuse; semantic novelty
remains the caller's responsibility.

## 9. Evidence model

### 9.1 Observation identity and fields

Every new observation needs a request-stable number because database IDs do not
exist until the atomic session transaction inserts rows. Add these fields to
`observations` and `ObservationInput`:

```sql
observation_no INTEGER NOT NULL CHECK (observation_no >= 1),
assessment_phase TEXT NOT NULL CHECK (assessment_phase IN (
  'historical',
  'cold_retrieval',
  'guided_practice',
  'immediate_retry',
  'transfer',
  'incidental'
)),
hint_level TEXT NOT NULL DEFAULT 'none' CHECK (hint_level IN (
  'none', 'indirect', 'direct', 'answer_shown'
)),
learner_effort TEXT CHECK (learner_effort IS NULL OR learner_effort IN (
  'effortless', 'some_effort', 'substantial_effort', 'unknown'
)),
evidence_source TEXT NOT NULL DEFAULT 'assistant' CHECK (evidence_source IN (
  'learner', 'assistant', 'legacy'
)),
UNIQUE(session_id, observation_no)
```

`observation_no` must be unique across the complete session payload, including
session-level, item-level, and attempt-level nested arrays. Validate this before
starting SQL writes.

Historical backfill: within each existing session, assign observation numbers
in ascending observation ID order, set `assessment_phase='historical'`,
`hint_level='none'`, `learner_effort=NULL`, and `evidence_source='legacy'`.
Keep role, outcome, evidence strength, severity, and text fields unchanged.

For new observations:

- cold retrieval is deliberate, unprompted evidence collected before feedback;
- guided practice follows instruction or scaffolding;
- immediate retry follows correction in the same learning episode;
- transfer uses a materially different cue or context;
- incidental evidence was not a declared target test;
- `role='targeted'` is required for cold retrieval, guided practice, immediate
  retry, and deliberate transfer;
- `evidence_strength` is required for cold retrieval, guided practice,
  immediate retry, and deliberate transfer; it may remain null for incidental
  evidence;
- `assessment_phase='incidental'` requires `role='incidental'`;
- `hint_level='answer_shown'` cannot be `outcome='correct'`; use
  `prompted_correct` if the learner subsequently produces it;
- `easy` must never be inferred solely from a clean transcript.

Enforce cross-field rules in Rust and cover them with tests; SQLite checks alone
are insufficient.

### 9.2 Attempts and actual timing

Add to `attempts` and `AttemptInput`:

```sql
actual_duration_milliseconds INTEGER
  CHECK (actual_duration_milliseconds IS NULL OR
         actual_duration_milliseconds > 0)
```

Keep `target_duration_seconds`. Actual duration must come from a timer or caller,
not transcript inference. Existing 24 attempts receive null actual duration and
retain null target duration.

### 9.3 Review-to-observation audit trail

Add an explicit evidence junction:

```sql
CREATE TABLE review_observations (
  review_id INTEGER NOT NULL REFERENCES weakness_reviews(id) ON DELETE CASCADE,
  observation_id INTEGER NOT NULL REFERENCES observations(id),
  evidence_role TEXT NOT NULL CHECK (evidence_role IN (
    'initial', 'supporting', 'contradicting', 'transfer'
  )),
  PRIMARY KEY (review_id, observation_id)
);

CREATE INDEX review_observations_observation_idx
  ON review_observations(observation_id, review_id);
```

Extend `ReviewInput` with:

```rust
evidence_observation_nos: Vec<u16>,
rating_rationale: Option<String>,
rating_source: RatingSource,
```

`RatingSource` is one of `learner`, `assistant_suggested_confirmed`, or
`assistant_suggested`. The last value is allowed for ratings 1 through 3 but not
for `easy`; `easy` requires learner confirmation and at least one linked initial
observation with `learner_effort='effortless'`.

Before inserting a review, validate:

1. all referenced observation numbers exist in the current session;
2. all referenced observations have the same weakness as the review;
3. at least one referenced observation is targeted cold retrieval with no hint;
4. exactly one earliest cold observation is tagged `initial` by the server;
5. later immediate retries are supporting evidence, not additional reviews;
6. the weakness is a declared practice-item target in the session;
7. there is at most one review per weakness/scheduler item per session;
8. the rationale is at most 1,000 characters;
9. the review's evidence strength equals the initial cold observation's
   evidence strength; `retrieval_mode` describes the task that elicited it;
10. existing 4,096-byte bounds on the optional evidence JSON remain.

The review rating should describe the earliest unprompted cold retrieval:

- incorrect or omitted initial retrieval normally means `again` even if an
  immediate retry succeeds;
- partially correct unprompted retrieval, or correct but genuinely effortful
  initial retrieval, means `hard`; a prompted response is not cold initial
  evidence and cannot by itself justify `hard` instead of `again`;
- independently correct initial retrieval means `good`;
- independently correct and explicitly effortless retrieval may mean `easy`.

The server validates evidence and the `easy` precondition but does not silently
rewrite an explicit non-easy rating. Store disagreements or unusual aggregation
decisions in `rating_rationale` and `evidence_json`.

## 10. Relational integrity corrections

### 10.1 Same-session foreign keys

The current schema can represent an observation whose `session_id` differs from
its attempt or practice item even though the live snapshot contains no such
rows. Rebuild the relevant tables with composite keys:

- add `UNIQUE(id, session_id)` to `practice_items` and `attempts`;
- make attempts reference `(practice_item_id, session_id)` to
  `practice_items(id, session_id)`;
- make observations reference `(attempt_id, session_id)` and
  `(practice_item_id, session_id)` to the corresponding composite keys.

If an observation has both an attempt and practice item, add a trigger or Rust
validation requiring that the attempt's `practice_item_id` equals the
observation's `practice_item_id`. Preserve the ability to store session-level
observations with both links null.

Add analogous application validation to `review_observations`: the review and
observation must have the same session and weakness. A trigger is optional if
the transaction code and tests enforce this, because those values are reached
through joins rather than stored redundantly.

### 10.2 Recent-practice item-number bug

`get_recent_practice` currently selects `attempts.practice_item_id` and returns
it under the JSON name `practice_item_no`. Fix the query by joining
`practice_items` and returning `practice_items.item_no`. Add a regression test in
which database ID and item number differ.

### 10.3 Normalize exercise types

Add:

```sql
CREATE TABLE exercise_types (
  key TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);
```

Add nullable `sessions.exercise_type_key` referencing this table; require it for
all new sessions. Preserve the old free-text `exercise_type` column for display
and historical compatibility. Backfill exact current values as follows:

| Existing value | Canonical key |
| --- | --- |
| `4-3-2` | `fluency_4_3_2` |
| `production drill` | `production_drill` |
| `translation_drill` | `translation_drill` |
| `DELE A2 oral microdrill` | `dele_a2_oral_microdrill` |
| `agreement/disagreement drill` | `agreement_disagreement_drill` |
| `guided conversation drill` | `guided_conversation` |

New API inputs should accept the canonical key. Continue returning the label or
legacy text for human readability.

### 10.4 Timestamp normalization

All new stored timestamps must be UTC RFC 3339 with `T`, millisecond precision,
and `Z`. `session_date` and due learning days remain `YYYY-MM-DD` values.

During migration, normalize exactly the 15 session, 22 attempt, and 135
observation timestamps ending in `+00`. Use Rust/Chrono parsing, convert to UTC,
and format with `SecondsFormat::Millis, true`. Do not alter semantic session
dates. Assert that all 17 + 24 + 162 affected table rows match canonical form
after migration. Do not use lexicographic ordering until normalization is
complete.

Replace the hand-written `Europe/Madrid` daylight-saving calculation with the
IANA timezone implementation from a compatible pinned `chrono-tz` dependency.
Parse `scheduler_config.timezone` as an IANA name and reject unknown zones.
Test learning-day calculation immediately before and after both Madrid clock
changes, including timestamps on each side of the 04:00 local cutoff.

## 11. Scheduler state and immutable parameters

### 11.1 Scheduler items

Move current cached state out of taxonomy/target rows into an explicit memory
unit table:

```sql
CREATE TABLE scheduler_items (
  id INTEGER PRIMARY KEY,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  track TEXT NOT NULL DEFAULT 'general' CHECK (track IN (
    'general', 'recognition', 'cued_production',
    'controlled_production', 'spontaneous_production'
  )),
  stability REAL CHECK (stability IS NULL OR stability > 0),
  difficulty REAL CHECK (
    difficulty IS NULL OR (difficulty >= 1 AND difficulty <= 10)
  ),
  due_learning_day TEXT,
  last_review_at TEXT,
  algorithm_version TEXT,
  parameter_set_id INTEGER REFERENCES scheduler_parameter_sets(id),
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(weakness_id, track),
  CHECK (
    (stability IS NULL AND difficulty IS NULL AND due_learning_day IS NULL AND
     last_review_at IS NULL AND algorithm_version IS NULL AND
     parameter_set_id IS NULL) OR
    (stability IS NOT NULL AND difficulty IS NOT NULL AND
     due_learning_day IS NOT NULL AND last_review_at IS NOT NULL AND
     algorithm_version IS NOT NULL AND parameter_set_id IS NOT NULL)
  )
);

CREATE INDEX scheduler_items_queue_idx
  ON scheduler_items(active, due_learning_day);
```

Create one active `general` item for each of the current 39 weaknesses, with null
state and due day. Keep the existing FSRS columns on `weaknesses` as deprecated
compatibility columns during one release, but stop updating them after the new
table is active. All queue and review code must use `scheduler_items`.

Parent concepts and collections must never create scheduler items. A future
retrieval-mode split creates new tracks only after explicit migration and must
not silently copy state.

### 11.2 Immutable parameter sets

The mutable singleton currently stores the only copy of the 21 parameters.
`weakness_reviews.parameters_version` alone cannot reproduce history after the
singleton is overwritten. Add:

```sql
CREATE TABLE scheduler_parameter_sets (
  id INTEGER PRIMARY KEY,
  algorithm TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)),
  source TEXT NOT NULL CHECK (source IN ('default','optimized')),
  training_summary_json TEXT CHECK (
    training_summary_json IS NULL OR json_valid(training_summary_json)
  ),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(algorithm, algorithm_version, parameters_json)
);
```

Because `scheduler_items` references this table, create parameter sets before
scheduler items in fresh schema and migration order.

Change `scheduler_config` to reference `active_parameter_set_id`. Keep desired
retention, timezone, cutoff, and updated timestamp in the singleton because they
are current policy rather than immutable model weights. Copy the exact current
parameter vector into parameter set 1 and point the config at it.

The rebuilt singleton is:

```sql
CREATE TABLE scheduler_config (
  id INTEGER PRIMARY KEY CHECK (id=1),
  active_parameter_set_id INTEGER NOT NULL
    REFERENCES scheduler_parameter_sets(id),
  desired_retention REAL NOT NULL CHECK (
    desired_retention >= 0.70 AND desired_retention <= 0.97
  ),
  timezone TEXT NOT NULL DEFAULT 'Europe/Madrid',
  day_cutoff_hour INTEGER NOT NULL DEFAULT 4 CHECK (
    day_cutoff_hour BETWEEN 0 AND 23
  ),
  updated_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
```

Rebuild `weakness_reviews` so every review references both `scheduler_item_id`
and immutable `parameter_set_id`. Retain `desired_retention` on each review.
Also add `review_learning_day`, `rating_source`, and `rating_rationale`.
`due_at` currently contains a date, not a timestamp; rename it to
`due_learning_day` in the rebuilt table and JSON API. The current review table
is empty, so no review rows require conversion.

Use this exact rebuilt review table:

```sql
CREATE TABLE weakness_reviews (
  id INTEGER PRIMARY KEY,
  scheduler_item_id INTEGER NOT NULL REFERENCES scheduler_items(id),
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  reviewed_at TEXT NOT NULL,
  review_learning_day TEXT NOT NULL,
  rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
  rating_source TEXT NOT NULL CHECK (rating_source IN (
    'learner', 'assistant_suggested_confirmed', 'assistant_suggested'
  )),
  rating_rationale TEXT,
  retrieval_mode TEXT NOT NULL CHECK (retrieval_mode IN (
    'recognition', 'cued_production', 'controlled_production',
    'spontaneous_production'
  )),
  evidence_strength TEXT NOT NULL CHECK (evidence_strength IN (
    'recognition', 'cued_production', 'controlled_production',
    'spontaneous_production'
  )),
  evidence_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(evidence_json)),
  elapsed_days INTEGER NOT NULL CHECK (elapsed_days >= 0),
  desired_retention REAL NOT NULL CHECK (
    desired_retention >= 0.70 AND desired_retention <= 0.97
  ),
  retrievability_before REAL CHECK (
    retrievability_before IS NULL OR
    (retrievability_before >= 0 AND retrievability_before <= 1)
  ),
  stability_before REAL CHECK (
    stability_before IS NULL OR stability_before > 0
  ),
  difficulty_before REAL CHECK (
    difficulty_before IS NULL OR
    (difficulty_before >= 1 AND difficulty_before <= 10)
  ),
  stability_after REAL NOT NULL CHECK (stability_after > 0),
  difficulty_after REAL NOT NULL CHECK (
    difficulty_after >= 1 AND difficulty_after <= 10
  ),
  scheduled_interval_days INTEGER NOT NULL CHECK (
    scheduled_interval_days >= 1
  ),
  due_learning_day TEXT NOT NULL,
  algorithm TEXT NOT NULL,
  algorithm_version TEXT NOT NULL,
  parameter_set_id INTEGER NOT NULL REFERENCES scheduler_parameter_sets(id),
  created_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, scheduler_item_id)
);

CREATE INDEX weakness_reviews_item_time_idx
  ON weakness_reviews(scheduler_item_id, reviewed_at);
```

Resolve the weakness through `scheduler_items.weakness_id`; do not duplicate it
on the review row. Validate in Rust that `algorithm` and `algorithm_version`
match the referenced immutable parameter set and that both after-state values
match the state cached on the scheduler item in the same transaction.

Do not optimize parameters now. The official `fsrs-rs` optimizer expects
histories from many items. Only deliberate linked reviews created under this
evidence model may enter a future training set; historical observations and
inferred ratings must be excluded.

## 12. Historical FSRS bootstrap and queue behavior

**Decision:** Preserve all 162 historical observations but create zero
historical `weakness_reviews`. Delete or permanently disable
`reconstruct_legacy_reviews`. Do not label unknown history as controlled
production, do not synthesize noon timestamps, and do not infer ratings from
mixed observation groups.

All 39 scheduler items start uninitialized. This must not make their ordering
alphabetical. For uninitialized targets, compute a deterministic onboarding
priority from historical evidence:

1. distinct session dates with an incorrect observation, descending;
2. unresolved recurrence, descending: one when there is no correct observation
   on a strictly later session date than the most recent incorrect date, else
   zero;
3. most recent incorrect session date, descending;
4. total incorrect observations, descending;
5. stable weakness key, ascending.

Count distinct dates, not raw rounds, so repeated 4-3-2 observations do not
dominate. As a migration/queue regression fixture, the initial leading targets
must begin with:

1. `preterite_completed_evaluation` — 4 error days, latest `2026-08-27`, 6
   incorrect observations;
2. `agreement_gender_number` — 4 error days, latest `2026-08-25`, 10 errors;
3. `para_que_subjunctive` — 3 error days, latest `2026-08-22`, 4 errors;
4. the 2-error-day group ordered by recency, error count, and key.

After initial sorting, diversify targets by the top-level ancestor of their
primary concept, not legacy category. Do not reorder targets with materially
different due priority merely for diversity.

For a normal six-exercise practice brief:

- select due initialized scheduler items first;
- introduce no more than two uninitialized targets unless explicit keys were
  requested;
- allocate at least two materially varied cues to a target when it will receive
  an FSRS rating;
- use incidental history and recent error examples for prompt design, not as
  scheduler state;
- once a target receives its first valid deliberate review, remove it from the
  onboarding ordering and use FSRS due/retrievability ordering.

## 13. Idempotency repair

Add to `recorded_requests`:

```sql
replayable INTEGER NOT NULL DEFAULT 1 CHECK (replayable IN (0,1))
```

For the exact two current rows, set `replayable=0`. Preserve both keys and
session links. If a caller presents one of these keys, return a specific domain
error stating that the legacy request record cannot be replayed; do not compare
its empty hash and do not return `{}` as a successful response.

For new rows:

- require `replayable=1`;
- require a 64-character lower-case SHA-256 hex request hash;
- require a valid, non-empty response JSON object;
- hash the canonical serialized request including observation numbers and
  review evidence references;
- preserve existing behavior: same key/same hash returns the stored response,
  same key/different hash is an error.

Use a rebuilt table if necessary to add strict checks for fresh data. Do not
delete the legacy rows merely to satisfy a check constraint; either permit
empty fields only when `replayable=0` or migrate them to a dedicated audit table.

## 14. Versioned migrations

Replace the single `schema_meta` insert-or-ignore mechanism with:

```sql
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  checksum TEXT NOT NULL,
  applied_at TEXT NOT NULL DEFAULT
    (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
```

The current code's `INSERT OR IGNORE schema_version=3` cannot reliably advance
an existing value. Keep `schema_meta` read-only for one compatibility release,
but make `schema_migrations` authoritative.

There are two initialization paths and they must not be mixed:

1. For a genuinely empty database, create the complete latest schema in
   dependency order, insert taxonomy/drill/exercise seeds, insert the default
   immutable parameter set and scheduler config, and record every baseline
   migration version. Snapshot-specific assertions do not run.
2. For the exact schema-3 production snapshot, run the preflight assertions and
   ordered migrations below before executing any latest-schema `CREATE TABLE IF
   NOT EXISTS` batch. This ordering prevents the current bug in which creating
   `scheduler_config` first makes historical data appear already migrated.

The SQLite migration runner is the only historical-data finalizer. It must
validate the exact schema-3 snapshot before changing it, must not use table
existence as a proxy for whether legacy data has been classified or scheduled,
and must preserve observations as historical while creating zero inferred
reviews.

Implement ordered Rust migrations, each with a fixed embedded SQL/checksum and
one transaction:

1. `003_baseline_snapshot`: record that the asserted source is schema 3;
2. `004_taxonomy_graph`: create schemes, concepts, graph, collections,
   normalized drill tables, and target classification fields; seed and map all
   39 targets;
3. `005_evidence_integrity`: rebuild item/attempt/observation tables as needed,
   assign observation numbers, add phases/source/effort/hints/timing/fingerprint,
   and normalize timestamps;
4. `006_scheduler_audit`: create immutable parameter sets and scheduler items,
   rebuild the empty review table, and add review evidence links;
5. `007_operational_cleanup`: normalize exercise types, mark legacy idempotency
   rows non-replayable, and add final indexes.

Do not infer whether a migration is needed from table existence. Read the
authoritative migration versions. Never run historical review reconstruction as
a side effect of opening a database.

SQLite table rebuilds must preserve IDs. Run `PRAGMA foreign_key_check` and
`PRAGMA integrity_check` after every migration in tests and after the complete
production migration. Be careful that changing `PRAGMA foreign_keys` cannot be
done effectively in the middle of a transaction; structure the migration
runner correctly rather than silently disabling enforcement.

## 15. Query and API changes

### 15.1 Existing read operations

Extend `get_learning_context`, `get_recent_practice`, `get_practice_brief`, and
`get_review_queue` output with a compact classification object:

```json
{
  "target_type": "grammatical_construction",
  "target_status": "active",
  "primary_concept": {
    "key": "form.grammar.syntax.subordination",
    "label": "Subordination"
  },
  "concepts": [
    {"key": "form.grammar.verb_system.mood.subjunctive", "role": "form"},
    {"key": "meaning.purpose", "role": "meaning"}
  ],
  "collections": []
}
```

Summary mode may omit secondary labels and definitions. Full mode includes
them. Bound every returned list.

Add optional filters where relevant:

- `concept_keys`: match direct links or descendants when explicitly requested;
- `scheme_keys`;
- `collection_keys`;
- `target_types`;
- retain `categories` for compatibility and mark it deprecated in tool
  descriptions.

Use parameterized SQL only. Resolve descendant filters with a recursive CTE and
a maximum requested depth. A target matching two selected concepts must appear
only once.

### 15.2 Practice planning

Replace category-based `diversify_rows` with primary-concept branch
diversification. Include selection explanations such as:

- `due FSRS review; predicted retrievability 0.84`;
- `uninitialized; errors on 4 distinct historical days`;
- `explicit collection target`;
- `classification pending`.

Return inherited drill recommendations with their stage and source concept.
Continue returning recent exact prompts and recent errors, but do not expose
answers before the learner responds.

### 15.3 Recording

Update request validation and SQL insertion in this order, within one
transaction:

1. validate idempotency and canonical request hash;
2. create or update explicitly supplied new weaknesses and classifications;
3. insert session and normalized exercise type;
4. insert practice items and fingerprints;
5. insert targets;
6. insert attempts and measured timing;
7. insert numbered observations while retaining a number-to-ID map;
8. validate reviews against that map and insert scheduler/review state;
9. insert `review_observations` links;
10. update first/last seen and target lifecycle where appropriate;
11. store the full response and commit.

Any validation or write failure rolls back every step.

## 16. Testing requirements

### 16.1 Exact snapshot migration test

Create a temporary copy of `data/aprendiendo-live.sqlite3`; never mutate the
checked-in snapshot. Open it through the new migration runner and assert:

- all preflight facts in section 2;
- original core row counts and IDs are unchanged;
- text fields are byte-for-byte unchanged except the explicitly normalized
  timestamp columns;
- 39 scheduler items exist and all are uninitialized;
- zero weakness reviews and zero review-observation links exist;
- all 39 active targets have exactly one primary concept;
- all mappings in section 6.2 exist exactly once;
- all 162 observations have unique per-session numbers and phase `historical`;
- exactly two recorded requests are non-replayable;
- all sessions have the expected canonical exercise type;
- all timestamps in the migrated tables use canonical UTC form;
- integrity and foreign-key checks pass.

### 16.2 Taxonomy tests

Test:

- exact-one-primary validation;
- direct and indirect broader-cycle rejection;
- requires-cycle rejection;
- canonical storage and bidirectional querying of symmetric relations;
- descendant filtering without duplicates;
- collection membership independent of hierarchy;
- concept deactivation blocked when used as an active primary concept;
- an unclassified future pronunciation, discourse, and lexical target;
- later reclassification without changing weakness or scheduler IDs;
- split via `supersedes` without state transfer.

### 16.3 Evidence tests

Test:

- observation numbers unique across all nested input locations;
- cold retrieval requires targeted/no-hint evidence;
- incidental phase/role consistency;
- review references only same-session, same-weakness observations;
- at least one cold initial observation per review;
- immediate retry linked as supporting rather than a second review;
- `easy` rejected without learner confirmation and effortless evidence;
- actual timing accepted when supplied and never inferred;
- review insertion, FSRS update, and evidence links roll back atomically;
- two items targeting one weakness still create at most one review;
- three 4-3-2 rounds remain one practice event and at most one review.

### 16.4 Schema-correction tests

Test:

- cross-session attempt/item/observation references fail at the database layer;
- `get_recent_practice.practice_item_no` returns the item number, not row ID;
- immutable parameter set 1 remains unchanged after activating parameter set 2;
- old reviews continue referencing parameter set 1;
- non-replayable legacy idempotency keys return the specific error;
- new idempotent replay behavior remains correct;
- migration application is idempotent and checksum mismatches are fatal.

### 16.5 Queue tests

Using the migrated snapshot, assert the onboarding order begins with the three
targets listed in section 12 before diversification. Verify that a normal brief
introduces at most two new targets, explicit-key requests can override the cap,
and an initialized due target outranks an uninitialized target.

## 17. Implementation sequence and completion criteria

### 17.1 Expected code locations

Keep responsibilities separated so `src/db.rs` does not become the only place
where graph, migration, evidence, and scheduler rules live:

- `sql/sqlite_schema.sql`: complete latest schema for an empty database;
- `src/migrations.rs` or `src/db/migrations.rs`: ordered migration registry,
  checksums, exact snapshot preflight, rebuild helpers, seeds, and postflight;
- `src/taxonomy.rs`: graph loading, cycle checks, ancestor/descendant queries,
  target classification, collections, and drill inheritance;
- `src/evidence.rs`: cross-field observation validation, rating-evidence
  validation, and observation-number mapping helpers;
- `src/fsrs_adapter.rs`: continue to own calls to the pinned official FSRS
  crate; load parameters through immutable parameter sets rather than global
  constants during scheduling;
- `src/model.rs`: new enums and request/response fields;
- `src/db.rs`: transaction orchestration and bounded relational queries;
- `src/server.rs`: MCP tools, field-size/list limits, and user-facing schemas;
- `src/bin/migrate.rs`: explicit SQLite migration entry point;
- `tests/protocol.rs` and focused module tests: tool schemas, migrations,
  validation, graph behavior, and regressions;
- `scripts/smoke_test.sh`: representative taxonomy read, classified practice
  brief, deliberate evidence recording, review queue, and replay.

This module split is recommended but not mandatory if repository conventions
strongly favor a different layout. The domain boundaries and tests are
mandatory.

### 17.2 Ordered work

Implement in this order:

1. Add migration infrastructure and exact snapshot fixture assertions.
2. Add taxonomy schema, seeds, mappings, read helpers, and taxonomy tests.
3. Add evidence fields, table rebuilds, composite foreign keys, and timestamp
   normalization.
4. Add immutable scheduler parameter sets, scheduler items, rebuilt reviews,
   and review-observation links.
5. Update Rust models and recording validation.
6. Update queues, practice planning, category replacement, and drill
   inheritance.
7. Add `get_taxonomy` and `upsert_concept`; extend existing tool schemas.
8. Fix recent-practice item numbering, exercise types, and idempotency.
9. Update `README.md`, `FSRS_PLAN.md` cross-references, smoke tests, and protocol
   tests.
10. Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
    `cargo test`, and `scripts/smoke_test.sh` against a temporary database.

The work is complete only when:

- the exact snapshot migrates without data loss or inferred reviews;
- all 39 current targets are mapped as specified;
- a future unclassified target can be recorded and later classified without
  changing its identity;
- the concept graph supports hierarchy, prerequisites, contrasts, and
  collections without conflating them;
- every new FSRS review has explicit cold evidence and immutable parameter
  provenance;
- queue ordering no longer degenerates to alphabetical ordering for the 39
  uninitialized targets;
- all tests and integrity checks pass.

## 18. Explicit non-goals

- Do not schedule concepts, parent nodes, collections, CEFR levels, drill types,
  or the general habit of doing 4-3-2.
- Do not import an entire external curriculum or linguistic ontology.
- Do not add a graph database, embeddings, or semantic-similarity service.
- Do not infer historical review ratings, retrieval modes, timing, severity, or
  hints.
- Do not optimize FSRS parameters from the current 162 observations.
- Do not split existing broad targets merely because a finer taxonomy is now
  available.
- Do not delete duplicate or anomalous historical rows.
- Do not make `category` authoritative again after graph classification exists.

## 19. Reference models

These sources explain the external organizational models used to shape this
plan. They are references, not schemas to import wholesale.

- W3C SKOS distinguishes concepts, concept schemes, broader/narrower hierarchy,
  associative relations, and collections:
  `https://www.w3.org/TR/skos-reference/`.
- The Instituto Cervantes PCIC organizes Spanish learning across separate
  inventories including grammar, pronunciation, orthography, communicative
  functions, pragmatics, genres, notions, and sociocultural content. It also
  explains why one phenomenon may appear in multiple inventories or levels:
  `https://cvc.cervantes.es/ensenanza/biblioteca_ele/plan_curricular/introduccion.htm`
  and
  `https://cvc.cervantes.es/ensenanza/biblioteca_ele/plan_curricular/indice.htm`.
- The CEFR Companion Volume supplies the broader linguistic,
  sociolinguistic, pragmatic, and communicative-activity framework:
  `https://www.coe.int/en/web/common-european-framework-reference-languages/cefr-companion-volume-and-its-language-versions`.
- Universal Dependencies supplies optional interoperable morphological and
  syntactic labels, including Spanish-specific guidance. It is not used as the
  pedagogical hierarchy:
  `https://universaldependencies.org/u/feat/index.html` and
  `https://universaldependencies.org/es/`.
