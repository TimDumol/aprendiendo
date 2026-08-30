# Live data analysis — 2026-08-29

## Scope and source

This analysis is based on a read-only SQLite snapshot taken from the running
Aprendiendo MCP deployment on `aries.timdumol.com` on 2026-08-29 at about
11:47 CEST.

The production database is located at:

```text
Host:      /opt/aprendiendo-mcp/app/data/aprendiendo.sqlite3
Container: /data/aprendiendo.sqlite3
```

The snapshot included the active SQLite WAL state. No remote data was modified.

## Current contents

| Table | Rows | Notes |
| --- | ---: | --- |
| `weaknesses` | 39 | All 39 are active |
| `sessions` | 17 | 2026-08-02 through 2026-08-28 |
| `attempts` | 24 | Narrative/response transcripts |
| `observations` | 162 | 82 incorrect, 80 correct |
| `practice_items` | 0 | New item-level model unused by imported history |
| `practice_item_targets` | 0 | No item-to-weakness links |
| `weakness_reviews` | 0 | No explicit FSRS reviews |
| `recorded_requests` | 2 | Both records have empty request hashes and `{}` responses |
| `scheduler_config` | 1 | FSRS-6, 90% desired retention |
| `schema_meta` | 1 | Schema version 3 |

### Weakness categories

There are 13 categories:

| Category | Count |
| --- | ---: |
| `collocation` | 7 |
| `clitics` | 5 |
| `prepositions` | 5 |
| `subjunctive` | 5 |
| `argument_structure` | 3 |
| `tense` | 3 |
| `agreement` | 2 |
| `aspect` | 2 |
| `fixed_phrase` | 2 |
| `vocabulary` | 2 |
| `lexical_grammar` | 1 |
| `ser_estar` | 1 |
| `verb_choice` | 1 |

Every weakness has a unique stable key, category, description, and target
pattern. All 39 have observations. The highest-volume/highest-error patterns
are:

- `agreement_gender_number`: 10 incorrect out of 15 observations.
- `aplicarse_clitics`: 5 incorrect out of 6.
- `preterite_completed_evaluation`: 6 incorrect out of 12.
- `aprovechar_para`: 3 incorrect out of 4.
- `reservar_asientos`: 3 incorrect out of 6.

Several low-volume weaknesses are currently 100% incorrect, but their samples
are too small to treat those rates as stable estimates.

### Historical session data

The 17 sessions cover oral microdrills, guided conversation, production drills,
translation drills, and 4-3-2 fluency sessions. The latest two sessions are
translation drills on 2026-08-28.

The 162 observations contain only `incorrect` and `correct` outcomes. All are
marked `role = incidental`; none has `evidence_strength` or `severity`.
Every observation has a produced response, while 142 have a correction and 153
have notes.

## Structural assessment

### What is sound

- SQLite integrity checks passed.
- No foreign-key orphans were found.
- No observation pointed to an attempt from another session.
- Weakness keys are unique and consistently formatted.
- The relational core (`sessions` → `attempts` → `observations` → `weaknesses`)
  is understandable and usable.
- The weakness catalog is already reasonably granular; a wholesale rewrite is
  unnecessary.

### Main problems

The production data is semantically half-migrated to the newer model:

1. Historical observations are not connected to `practice_items`. There are 31
   session-level observations and 131 attempt-linked observations, but no exact
   prompt/item linkage. This limits prompt avoidance, item-level reporting, and
   deliberate review tracking.
2. All FSRS state is uninitialized. The database has legacy `due_date`,
   `interval_days`, `ease_factor`, `repetitions`, and `lapses` values, but no
   `weakness_reviews` and no FSRS stability/difficulty/due state. The current
   adapter therefore treats all 39 weaknesses as new FSRS cards, despite their
   legacy due dates.
3. The imported history does not distinguish targeted from incidental errors.
   This is acceptable as historical provenance, but it should not be interpreted
   as deliberate review evidence.
4. `recommended_drill_types` is empty for all 39 weaknesses. The server has
   fallback drill selection, so this is functional but not explicit data.
5. The two existing `recorded_requests` rows satisfy the SQL schema only because
   migration defaults allow empty hashes and empty responses. They cannot safely
   validate an idempotent replay without the original payload hash.
6. Timestamps use mixed textual formats: older records use a space and `+00`
   suffix, while newer records use ISO `T...Z` form. Date-only session fields
   are consistent, but timestamp normalization would improve sorting and
   interoperability.
7. `first_seen` for `adverb_invariable_modifier` predates the first linked
   observation. This is probably catalog-discovery time rather than an error,
   but the distinction should be explicit.

## Proposed taxonomy

Use one primary hierarchy for aggregation while retaining the existing weakness
keys as leaf nodes. Do not merge leaves merely because they share a category;
their review histories may eventually need separate FSRS schedules.

```text
Spanish learning
├── Morphosyntax
│   ├── Agreement
│   ├── Clitics and object pronouns
│   ├── Tense and aspect
│   ├── Subordination and subjunctive
│   └── Copular constructions
├── Valency and grammatical frames
│   ├── Argument structure
│   ├── Prepositional frames
│   └── Verb complementation
└── Lexis and phraseology
    ├── Collocations
    ├── Fixed phrases
    └── Lexical choice
```

### Leaf mapping

#### Morphosyntax

- **Agreement:** `agreement_gender_number`, `adverb_invariable_modifier`
- **Clitics and object pronouns:** `aplicarse_clitics`, `hacer_pensar`,
  `double_object_clitics`, `decirselo_form`, `prometer_infinitive_clitics`
- **Tense and aspect:** `preterite_imperfect_background`,
  `preterite_completed_evaluation`, `preterite_ayer`,
  `duration_llevar_gerund`, `iba_a_infinitive`
- **Subordination and subjunctive:** `reported_command_past`,
  `reported_influence_past`, `para_que_subjunctive`,
  `antes_de_que_subjunctive`, `past_nonspecific_relative_subjunctive`
- **Copular constructions:** `ser_estar_event_result`

#### Valency and grammatical frames

- **Argument structure:** `acompanar_direct_object`, `impedir_a_alguien`,
  `doler_gustar_structure`
- **Prepositional frames:** `body_part_prepositions`, `caused_by`,
  `despedirse_de`, `space_para_piernas`, `salir_de_place_event`
- **Verb complementation:** `tratar_de_topic`, `aprovechar_para`,
  `faltar_tiempo`

#### Lexis and phraseology

- **Collocations:** `club_de_lectura`, `dar_estres_energia`, `cometer_error`,
  `reservar_asientos`, `hinchazon_disminuir`
- **Fixed phrases:** `cuanto_antes`, `durante_todo_periodo`
- **Lexical choice:** `paraguas_vocab`, `farmaceutico_vocab`,
  `queria_querria_quisiera`

### Useful intermediate parents

Some related leaves should share a parent without being merged:

```text
subordination.reported_past
├── reported_command_past
└── reported_influence_past

tense_aspect.preterite_selection
├── preterite_imperfect_background
├── preterite_completed_evaluation
└── preterite_ayer

clitics.object_pronouns
├── double_object_clitics
├── decirselo_form
└── prometer_infinitive_clitics
```

`queria_querria_quisiera` currently bundles several forms, and
`ser_estar_event_result` bundles two related contrasts. Keep them together until
there is enough new evidence to show that their triggers and review schedules
diverge. Split them later only if the correction patterns are demonstrably
different.

## Recommended implementation shape

Keep `weaknesses.key` stable and add a normalized taxonomy layer rather than
renaming existing keys:

```text
skill_nodes
  key              primary key
  parent_key       nullable self-reference
  node_type        parent | leaf
  label
  description
  active

weaknesses
  key              foreign key to a leaf skill node
  ... existing review/catalog fields ...
```

Alternatively, a nullable `parent_key` and `taxonomy_path` on `weaknesses` is
adequate for the current size. A separate table becomes preferable once parent
nodes need their own descriptions, ordering, or localization.

The scheduler should operate only on leaf weaknesses. Parent nodes should
aggregate child observations and help select practice targets. `drill_type`,
session topic, CEFR level, and exercise format should remain independent fields;
they describe the intervention or context, not the weakness taxonomy.

## Priority order

1. Treat the imported rows explicitly as historical/legacy evidence.
2. Decide whether and how to initialize FSRS from the legacy history; do not
   silently equate legacy due dates with FSRS state.
3. Record new sessions with practice items, target links, observation roles, and
   explicit reviews.
4. Add the parent taxonomy and aggregate reporting while preserving stable leaf
   keys.
5. Normalize timestamps and reconcile the incomplete idempotency records.

