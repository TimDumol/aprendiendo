PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS sessions (
  id INTEGER PRIMARY KEY,
  session_date TEXT NOT NULL DEFAULT (date('now')),
  exercise_type_key TEXT NOT NULL CHECK (exercise_type_key IN (
    'fluency_4_3_2', 'production_drill', 'translation_drill',
    'dele_a2_oral_microdrill', 'agreement_disagreement_drill',
    'guided_conversation'
  )),
  topic TEXT,
  task_ref TEXT,
  exercise_instance_id TEXT,
  stimulus_identity TEXT,
  source_event_id TEXT,
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS weaknesses (
  id INTEGER PRIMARY KEY,
  key TEXT NOT NULL UNIQUE,
  category TEXT NOT NULL,
  description TEXT NOT NULL,
  target_pattern TEXT,
  recommended_drill_types TEXT CHECK (recommended_drill_types IS NULL OR json_valid(recommended_drill_types)),
  first_seen TEXT,
  last_seen TEXT,
  due_date TEXT NOT NULL DEFAULT (date('now')),
  interval_days INTEGER NOT NULL DEFAULT 0 CHECK (interval_days >= 0),
  ease_factor REAL NOT NULL DEFAULT 2.5 CHECK (ease_factor >= 1.3),
  repetitions INTEGER NOT NULL DEFAULT 0 CHECK (repetitions >= 0),
  lapses INTEGER NOT NULL DEFAULT 0 CHECK (lapses >= 0),
  last_reviewed TEXT,
  fsrs_stability REAL CHECK (fsrs_stability IS NULL OR fsrs_stability > 0),
  fsrs_difficulty REAL CHECK (fsrs_difficulty IS NULL OR (fsrs_difficulty >= 1 AND fsrs_difficulty <= 10)),
  fsrs_due_at TEXT,
  fsrs_last_review_at TEXT,
  fsrs_algorithm_version TEXT,
  fsrs_parameters_version INTEGER,
  target_type TEXT NOT NULL DEFAULT 'grammatical_construction' CHECK (target_type IN (
    'grammatical_construction', 'lexical_item', 'lexical_chunk',
    'form_meaning_contrast', 'pronunciation', 'orthography',
    'discourse_strategy', 'sociopragmatic_choice'
  )),
  target_status TEXT NOT NULL DEFAULT 'active' CHECK (target_status IN (
    'candidate', 'active', 'suspended', 'retired', 'merged'
  )),
  classification_pending INTEGER NOT NULL DEFAULT 0 CHECK (classification_pending IN (0,1)),
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS concept_schemes (
  id INTEGER PRIMARY KEY,
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  description TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE TABLE IF NOT EXISTS concepts (
  id INTEGER PRIMARY KEY,
  scheme_id INTEGER NOT NULL REFERENCES concept_schemes(id),
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  definition TEXT,
  source_uri TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS concepts_scheme_idx ON concepts(scheme_id, active, sort_order, key);

CREATE TABLE IF NOT EXISTS concept_edges (
  subject_id INTEGER NOT NULL REFERENCES concepts(id),
  predicate TEXT NOT NULL CHECK (predicate IN ('broader','requires','contrasts_with','related')),
  object_id INTEGER NOT NULL REFERENCES concepts(id),
  provenance TEXT NOT NULL DEFAULT 'curated' CHECK (provenance IN ('curated','external','inferred')),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (subject_id, predicate, object_id), CHECK (subject_id <> object_id)
);
CREATE INDEX IF NOT EXISTS concept_edges_object_idx ON concept_edges(object_id, predicate, subject_id);

CREATE TABLE IF NOT EXISTS weakness_concepts (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  concept_id INTEGER NOT NULL REFERENCES concepts(id),
  role TEXT NOT NULL CHECK (role IN ('primary','form','meaning','function','context','error_source','curriculum')),
  provenance TEXT NOT NULL DEFAULT 'curated' CHECK (provenance IN ('curated','external','inferred')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (weakness_id, concept_id, role)
);
CREATE UNIQUE INDEX IF NOT EXISTS weakness_one_primary_concept_idx ON weakness_concepts(weakness_id) WHERE role='primary';
CREATE INDEX IF NOT EXISTS weakness_concepts_concept_idx ON weakness_concepts(concept_id, role, weakness_id);

CREATE TABLE IF NOT EXISTS weakness_relations (
  subject_id INTEGER NOT NULL REFERENCES weaknesses(id),
  predicate TEXT NOT NULL CHECK (predicate IN ('confusable_with','variant_of','supersedes','practice_together')),
  object_id INTEGER NOT NULL REFERENCES weaknesses(id),
  notes TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (subject_id, predicate, object_id), CHECK (subject_id <> object_id)
);
CREATE INDEX IF NOT EXISTS weakness_relations_object_idx ON weakness_relations(object_id, predicate, subject_id);

CREATE TABLE IF NOT EXISTS collections (
  id INTEGER PRIMARY KEY,
  key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  collection_type TEXT NOT NULL CHECK (collection_type IN ('goal','curriculum','topic','campaign')),
  description TEXT,
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS collection_members (
  collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id), position INTEGER, notes TEXT,
  PRIMARY KEY (collection_id, weakness_id)
);

CREATE TABLE IF NOT EXISTS curriculum_mappings (
  id INTEGER PRIMARY KEY,
  weakness_id INTEGER REFERENCES weaknesses(id), concept_id INTEGER REFERENCES concepts(id),
  framework TEXT NOT NULL CHECK (framework IN ('PCIC','CEFR','UD','RAE')),
  external_key TEXT NOT NULL, external_uri TEXT,
  relation TEXT NOT NULL CHECK (relation IN ('exact','close','broader','narrower','related')),
  level_min TEXT, level_max TEXT, notes TEXT,
  CHECK ((weakness_id IS NULL) <> (concept_id IS NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS curriculum_mapping_weakness_idx ON curriculum_mappings(weakness_id, framework, external_key) WHERE weakness_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS curriculum_mapping_concept_idx ON curriculum_mappings(concept_id, framework, external_key) WHERE concept_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS concept_drill_recommendations (
  concept_id INTEGER NOT NULL REFERENCES concepts(id),
  drill_type TEXT NOT NULL CHECK (drill_type IN ('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
  stage TEXT NOT NULL CHECK (stage IN ('recognition','controlled','transfer','fluency')),
  weight REAL NOT NULL CHECK (weight > 0 AND weight <= 1), PRIMARY KEY (concept_id, drill_type, stage)
);
CREATE TABLE IF NOT EXISTS weakness_drill_overrides (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  drill_type TEXT NOT NULL CHECK (drill_type IN ('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
  stage TEXT NOT NULL CHECK (stage IN ('recognition','controlled','transfer','fluency')),
  weight REAL NOT NULL CHECK (weight >= 0 AND weight <= 1), PRIMARY KEY (weakness_id, drill_type, stage)
);

CREATE TABLE IF NOT EXISTS activity_types (
  key TEXT PRIMARY KEY,
  label TEXT NOT NULL,
  interaction_mode TEXT NOT NULL CHECK (interaction_mode IN (
    'single_response', 'sprint', 'multi_turn'
  )),
  active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1))
);

CREATE TABLE IF NOT EXISTS activity_runs (
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
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, run_no),
  UNIQUE(id, session_id),
  CHECK (actual_duration_milliseconds IS NULL OR timing_source IS NOT NULL)
);
CREATE INDEX IF NOT EXISTS activity_runs_type_session_idx
  ON activity_runs(activity_type_key, session_id);

CREATE TABLE IF NOT EXISTS practice_items (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  item_no INTEGER NOT NULL CHECK (item_no >= 1),
  drill_type TEXT NOT NULL CHECK (drill_type IN ('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
  prompt TEXT NOT NULL, prompt_fingerprint TEXT,
  response TEXT, corrected_response TEXT, reference_answer TEXT, feedback TEXT,
  outcome TEXT CHECK (outcome IS NULL OR outcome IN ('incorrect','partially_correct','correct','omitted')),
  activity_run_id INTEGER,
  item_phase TEXT NOT NULL DEFAULT 'initial' CHECK (item_phase IN ('initial','follow_up','complication')),
  effective_status TEXT NOT NULL DEFAULT 'active' CHECK (effective_status IN ('active','retracted','disputed')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  computed_word_count INTEGER CHECK (computed_word_count IS NULL OR computed_word_count >= 0),
  word_counter_version TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(id, session_id), UNIQUE(session_id, item_no),
  FOREIGN KEY (activity_run_id, session_id)
    REFERENCES activity_runs(id, session_id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS practice_item_targets (
  practice_item_id INTEGER NOT NULL REFERENCES practice_items(id) ON DELETE CASCADE,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id), PRIMARY KEY (practice_item_id, weakness_id)
);

CREATE TABLE IF NOT EXISTS attempts (
  id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1), practice_item_id INTEGER,
  transcript TEXT NOT NULL, target_duration_seconds INTEGER CHECK (target_duration_seconds IS NULL OR target_duration_seconds > 0),
  actual_duration_milliseconds INTEGER CHECK (actual_duration_milliseconds IS NULL OR actual_duration_milliseconds > 0),
  response_mode TEXT CHECK (response_mode IS NULL OR response_mode IN ('typed','spoken_transcript')),
  response_latency_milliseconds INTEGER CHECK (response_latency_milliseconds IS NULL OR response_latency_milliseconds > 0),
  timing_source TEXT CHECK (timing_source IS NULL OR timing_source IN ('learner_reported','external_timer','client_measured')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(id, session_id), UNIQUE(session_id, attempt_no),
  CHECK (response_latency_milliseconds IS NULL OR timing_source IS NOT NULL),
  FOREIGN KEY (practice_item_id, session_id) REFERENCES practice_items(id, session_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS activity_stimuli (
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
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(activity_run_id, stimulus_no),
  CHECK (
    (content_text IS NOT NULL AND length(trim(content_text)) > 0)
    OR (source_uri IS NOT NULL AND length(trim(source_uri)) > 0)
  )
);
CREATE INDEX IF NOT EXISTS activity_stimuli_run_idx
  ON activity_stimuli(activity_run_id, stimulus_no);

CREATE TABLE IF NOT EXISTS attempt_reflections (
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
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(attempt_id, reflection_no)
);
CREATE INDEX IF NOT EXISTS attempt_reflections_attempt_idx
  ON attempt_reflections(attempt_id, reflection_no);

CREATE TABLE IF NOT EXISTS session_findings (
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  practice_item_id INTEGER,
  attempt_id INTEGER,
  assessment_kind TEXT NOT NULL CHECK (assessment_kind IN (
    'error', 'awkward', 'regional_variant', 'stylistic_improvement', 'accepted'
  )),
  original TEXT NOT NULL,
  suggestion TEXT,
  note TEXT,
  effective_status TEXT NOT NULL DEFAULT 'active' CHECK (effective_status IN ('active','retracted','disputed')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (practice_item_id, session_id)
    REFERENCES practice_items(id, session_id) ON DELETE CASCADE,
  FOREIGN KEY (attempt_id, session_id)
    REFERENCES attempts(id, session_id) ON DELETE CASCADE,
  CHECK (practice_item_id IS NOT NULL OR attempt_id IS NOT NULL OR original <> '')
);
CREATE INDEX IF NOT EXISTS session_findings_session_idx
  ON session_findings(session_id, id);
CREATE INDEX IF NOT EXISTS session_findings_kind_idx
  ON session_findings(session_id, assessment_kind);

CREATE TABLE IF NOT EXISTS observations (
  id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  attempt_id INTEGER, practice_item_id INTEGER, weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  observation_no INTEGER NOT NULL CHECK (observation_no >= 1),
  outcome TEXT NOT NULL CHECK (outcome IN ('incorrect','partially_correct','correct','prompted_correct','omitted')),
  role TEXT NOT NULL DEFAULT 'incidental' CHECK (role IN ('targeted','incidental')),
  assessment_phase TEXT NOT NULL CHECK (assessment_phase IN ('historical','cold_retrieval','guided_practice','immediate_retry','transfer','incidental')),
  hint_level TEXT NOT NULL DEFAULT 'none' CHECK (hint_level IN ('none','indirect','direct','answer_shown')),
  learner_effort TEXT CHECK (learner_effort IS NULL OR learner_effort IN ('effortless','some_effort','substantial_effort','unknown')),
  evidence_source TEXT NOT NULL DEFAULT 'assistant' CHECK (evidence_source IN ('learner','assistant','legacy')),
  evidence_strength TEXT CHECK (evidence_strength IS NULL OR evidence_strength IN ('recognition','cued_production','controlled_production','spontaneous_production')),
  severity TEXT CHECK (severity IS NULL OR severity IN ('minor','meaning_affecting','blocking')),
  produced TEXT, correction TEXT, error_span TEXT, notes TEXT,
  effective_status TEXT NOT NULL DEFAULT 'active' CHECK (effective_status IN ('active','retracted','disputed')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(session_id, observation_no),
  FOREIGN KEY (attempt_id, session_id) REFERENCES attempts(id, session_id) ON DELETE CASCADE,
  FOREIGN KEY (practice_item_id, session_id) REFERENCES practice_items(id, session_id) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS observations_same_item_trigger
BEFORE INSERT ON observations
WHEN NEW.attempt_id IS NOT NULL AND NEW.practice_item_id IS NOT NULL
 AND (SELECT practice_item_id FROM attempts WHERE id=NEW.attempt_id AND session_id=NEW.session_id) IS NOT NEW.practice_item_id
BEGIN SELECT RAISE(ABORT, 'observation attempt and practice item do not match'); END;

CREATE TABLE IF NOT EXISTS scheduler_parameter_sets (
  id INTEGER PRIMARY KEY, algorithm TEXT NOT NULL, algorithm_version TEXT NOT NULL,
  parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)), source TEXT NOT NULL CHECK (source IN ('default','optimized')),
  training_summary_json TEXT CHECK (training_summary_json IS NULL OR json_valid(training_summary_json)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  UNIQUE(algorithm, algorithm_version, parameters_json)
);
CREATE TABLE IF NOT EXISTS scheduler_items (
  id INTEGER PRIMARY KEY, weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  track TEXT NOT NULL DEFAULT 'general' CHECK (track IN ('general','recognition','cued_production','controlled_production','spontaneous_production')),
  stability REAL CHECK (stability IS NULL OR stability > 0), difficulty REAL CHECK (difficulty IS NULL OR (difficulty >= 1 AND difficulty <= 10)),
  due_learning_day TEXT, last_review_at TEXT, algorithm_version TEXT,
  parameter_set_id INTEGER REFERENCES scheduler_parameter_sets(id), active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(weakness_id, track),
  CHECK ((stability IS NULL AND difficulty IS NULL AND due_learning_day IS NULL AND last_review_at IS NULL AND algorithm_version IS NULL AND parameter_set_id IS NULL) OR (stability IS NOT NULL AND difficulty IS NOT NULL AND due_learning_day IS NOT NULL AND last_review_at IS NOT NULL AND algorithm_version IS NOT NULL AND parameter_set_id IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS scheduler_items_queue_idx ON scheduler_items(active, due_learning_day);

CREATE TABLE IF NOT EXISTS scheduler_config (
  id INTEGER PRIMARY KEY CHECK (id=1), active_parameter_set_id INTEGER NOT NULL REFERENCES scheduler_parameter_sets(id),
  desired_retention REAL NOT NULL CHECK (desired_retention >= 0.70 AND desired_retention <= 0.97),
  timezone TEXT NOT NULL DEFAULT 'Europe/Madrid', day_cutoff_hour INTEGER NOT NULL DEFAULT 4 CHECK (day_cutoff_hour BETWEEN 0 AND 23),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS weakness_reviews (
  id INTEGER PRIMARY KEY, scheduler_item_id INTEGER NOT NULL REFERENCES scheduler_items(id), session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  reviewed_at TEXT NOT NULL, review_learning_day TEXT NOT NULL, rating INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
  rating_source TEXT NOT NULL CHECK (rating_source IN ('learner','assistant_suggested_confirmed','assistant_suggested')), rating_rationale TEXT,
  retrieval_mode TEXT NOT NULL CHECK (retrieval_mode IN ('recognition','cued_production','controlled_production','spontaneous_production')),
  evidence_strength TEXT NOT NULL CHECK (evidence_strength IN ('recognition','cued_production','controlled_production','spontaneous_production')),
  evidence_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(evidence_json)), elapsed_days INTEGER NOT NULL CHECK (elapsed_days >= 0),
  desired_retention REAL NOT NULL CHECK (desired_retention >= 0.70 AND desired_retention <= 0.97), retrievability_before REAL CHECK (retrievability_before IS NULL OR (retrievability_before >= 0 AND retrievability_before <= 1)),
  stability_before REAL CHECK (stability_before IS NULL OR stability_before > 0), difficulty_before REAL CHECK (difficulty_before IS NULL OR (difficulty_before >= 1 AND difficulty_before <= 10)),
  stability_after REAL NOT NULL CHECK (stability_after > 0), difficulty_after REAL NOT NULL CHECK (difficulty_after >= 1 AND difficulty_after <= 10), scheduled_interval_days INTEGER NOT NULL CHECK (scheduled_interval_days >= 1),
  due_learning_day TEXT NOT NULL, algorithm TEXT NOT NULL, algorithm_version TEXT NOT NULL, parameter_set_id INTEGER NOT NULL REFERENCES scheduler_parameter_sets(id),
  effective_status TEXT NOT NULL DEFAULT 'active' CHECK (effective_status IN ('active','invalidated','disputed')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(session_id, scheduler_item_id)
);
CREATE INDEX IF NOT EXISTS weakness_reviews_item_time_idx ON weakness_reviews(scheduler_item_id, reviewed_at);
CREATE TABLE IF NOT EXISTS review_observations (
  review_id INTEGER NOT NULL REFERENCES weakness_reviews(id) ON DELETE CASCADE, observation_id INTEGER NOT NULL REFERENCES observations(id),
  evidence_role TEXT NOT NULL CHECK (evidence_role IN ('initial','supporting','contradicting','transfer')), PRIMARY KEY (review_id, observation_id)
);
CREATE INDEX IF NOT EXISTS review_observations_observation_idx ON review_observations(observation_id, review_id);

CREATE TABLE IF NOT EXISTS recorded_requests (
  idempotency_key TEXT PRIMARY KEY, session_id INTEGER NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
  request_hash TEXT NOT NULL, response_json TEXT NOT NULL CHECK (json_valid(response_json) AND json_type(response_json)='object'),
  replayable INTEGER NOT NULL DEFAULT 1 CHECK (replayable IN (0,1)), created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  CHECK (replayable=0 OR (length(request_hash)=64 AND request_hash NOT GLOB '*[^0-9a-f]*'))
);
CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL,
  applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS attempts_session_id_idx ON attempts(session_id);
CREATE INDEX IF NOT EXISTS observations_session_id_idx ON observations(session_id);
CREATE INDEX IF NOT EXISTS observations_attempt_id_idx ON observations(attempt_id);
CREATE INDEX IF NOT EXISTS observations_weakness_id_idx ON observations(weakness_id);
CREATE INDEX IF NOT EXISTS practice_items_session_id_idx ON practice_items(session_id);
CREATE INDEX IF NOT EXISTS practice_items_activity_run_idx ON practice_items(activity_run_id, item_no);
CREATE INDEX IF NOT EXISTS practice_item_targets_weakness_idx ON practice_item_targets(weakness_id, practice_item_id);
CREATE INDEX IF NOT EXISTS prompt_fingerprint_created_idx ON practice_items(prompt_fingerprint, created_at);
CREATE INDEX IF NOT EXISTS attempts_response_mode_session_idx ON attempts(response_mode, session_id);
CREATE INDEX IF NOT EXISTS sessions_task_ref_idx ON sessions(task_ref, session_date DESC, id DESC);

CREATE TABLE IF NOT EXISTS audit_runs (
  audit_run_id TEXT PRIMARY KEY,
  snapshot_boundary TEXT NOT NULL,
  snapshot_hash TEXT NOT NULL,
  model_id TEXT,
  prompt_version TEXT,
  scope_json TEXT NOT NULL DEFAULT '{}',
  started_at TEXT NOT NULL,
  ended_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('planned','running','completed','failed','partial')),
  records_examined INTEGER NOT NULL DEFAULT 0 CHECK(records_examined >= 0),
  proposed_count INTEGER NOT NULL DEFAULT 0 CHECK(proposed_count >= 0),
  applied_count INTEGER NOT NULL DEFAULT 0 CHECK(applied_count >= 0),
  rejected_count INTEGER NOT NULL DEFAULT 0 CHECK(rejected_count >= 0),
  disputed_count INTEGER NOT NULL DEFAULT 0 CHECK(disputed_count >= 0),
  failure_count INTEGER NOT NULL DEFAULT 0 CHECK(failure_count >= 0),
  latest_successful_boundary TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS data_amendments (
  audit_run_id TEXT NOT NULL REFERENCES audit_runs(audit_run_id),
  proposal_id TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  expected_revision INTEGER NOT NULL CHECK(expected_revision >= 1),
  before_hash TEXT NOT NULL,
  operation TEXT NOT NULL,
  before_json TEXT NOT NULL CHECK(json_valid(before_json)),
  after_json TEXT NOT NULL CHECK(json_valid(after_json)),
  source_refs_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(source_refs_json)),
  reason TEXT NOT NULL,
  issue_class TEXT NOT NULL,
  certainty TEXT NOT NULL CHECK(certainty IN ('supported','disputed','insufficient_evidence')),
  reviewer TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  reviewed_at TEXT NOT NULL,
  dependencies_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(dependencies_json)),
  affected_consumers_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(affected_consumers_json)),
  predecessor_revision INTEGER NOT NULL CHECK(predecessor_revision >= 1),
  status TEXT NOT NULL CHECK(status IN ('proposed','applied','rejected','disputed','reverted')),
  applied_at TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  error TEXT,
  PRIMARY KEY(audit_run_id, proposal_id)
);
CREATE INDEX IF NOT EXISTS data_amendments_entity_idx ON data_amendments(entity_type, entity_id, created_at);
CREATE INDEX IF NOT EXISTS data_amendments_run_status_idx ON data_amendments(audit_run_id, status);
CREATE TABLE IF NOT EXISTS effective_summaries (
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  summary_key TEXT NOT NULL,
  summary_json TEXT NOT NULL CHECK(json_valid(summary_json)),
  revision INTEGER NOT NULL CHECK(revision >= 1),
  updated_at TEXT NOT NULL,
  PRIMARY KEY(session_id, summary_key)
);
CREATE TABLE IF NOT EXISTS learner_goals (
  id INTEGER PRIMARY KEY CHECK(id=1), exam_key TEXT NOT NULL, target_date TEXT, oral_date TEXT,
  timezone TEXT NOT NULL, source TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS exam_specifications (
  exam_key TEXT NOT NULL, specification_version TEXT NOT NULL, source_url TEXT NOT NULL,
  retrieved_at TEXT NOT NULL, config_json TEXT NOT NULL CHECK(json_valid(config_json) AND json_type(config_json)='object'),
  PRIMARY KEY(exam_key, specification_version)
);
CREATE TABLE IF NOT EXISTS task_requirements (
  id INTEGER PRIMARY KEY, session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
  practice_item_id INTEGER, exam_key TEXT, specification_version TEXT, skill TEXT NOT NULL, task_key TEXT NOT NULL,
  prompt_hash TEXT, requested_points_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(requested_points_json)),
  word_min INTEGER CHECK(word_min IS NULL OR word_min >= 0), word_max INTEGER CHECK(word_max IS NULL OR word_max >= word_min),
  time_budget_seconds INTEGER CHECK(time_budget_seconds IS NULL OR time_budget_seconds > 0),
  preparation_budget_seconds INTEGER CHECK(preparation_budget_seconds IS NULL OR preparation_budget_seconds > 0),
  source TEXT NOT NULL CHECK(source IN ('official','teacher_created','micropractice')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (practice_item_id, session_id) REFERENCES practice_items(id, session_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS task_requirements_item_idx ON task_requirements(practice_item_id, session_id);
CREATE TABLE IF NOT EXISTS requested_point_assessments (
  id INTEGER PRIMARY KEY, attempt_id INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
  point_key TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('met','partial','missing','unclear')),
  supporting_excerpt TEXT, reason TEXT, evaluator TEXT NOT NULL, rubric_version TEXT, assessed_at TEXT NOT NULL,
  retrospective INTEGER NOT NULL DEFAULT 0 CHECK(retrospective IN (0,1)), UNIQUE(attempt_id, point_key)
);
CREATE TABLE IF NOT EXISTS audit_checkpoints (
  audit_run_id TEXT NOT NULL REFERENCES audit_runs(audit_run_id) ON DELETE CASCADE,
  unit_key TEXT NOT NULL, source_hash TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('pending','completed','failed')),
  error TEXT, updated_at TEXT NOT NULL, PRIMARY KEY(audit_run_id, unit_key)
);
CREATE UNIQUE INDEX IF NOT EXISTS sessions_source_event_idx ON sessions(source_event_id) WHERE source_event_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS observations_effective_idx ON observations(effective_status, weakness_id, session_id);
CREATE INDEX IF NOT EXISTS session_findings_effective_idx ON session_findings(effective_status, session_id, id);
CREATE INDEX IF NOT EXISTS weakness_reviews_effective_idx ON weakness_reviews(effective_status, scheduler_item_id, reviewed_at);
