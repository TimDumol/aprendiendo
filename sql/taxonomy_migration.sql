CREATE TABLE IF NOT EXISTS concept_schemes (
  id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE, label TEXT NOT NULL, description TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1 CHECK(active IN(0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS concepts (
  id INTEGER PRIMARY KEY, scheme_id INTEGER NOT NULL REFERENCES concept_schemes(id), key TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL, definition TEXT, source_uri TEXT, sort_order INTEGER NOT NULL DEFAULT 0,
  active INTEGER NOT NULL DEFAULT 1 CHECK(active IN(0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS concepts_scheme_idx ON concepts(scheme_id,active,sort_order,key);
CREATE TABLE IF NOT EXISTS concept_edges (
  subject_id INTEGER NOT NULL REFERENCES concepts(id), predicate TEXT NOT NULL CHECK(predicate IN('broader','requires','contrasts_with','related')),
  object_id INTEGER NOT NULL REFERENCES concepts(id), provenance TEXT NOT NULL DEFAULT 'curated' CHECK(provenance IN('curated','external','inferred')),
  notes TEXT, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY(subject_id,predicate,object_id), CHECK(subject_id<>object_id)
);
CREATE INDEX IF NOT EXISTS concept_edges_object_idx ON concept_edges(object_id,predicate,subject_id);
CREATE TABLE IF NOT EXISTS weakness_concepts (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id), concept_id INTEGER NOT NULL REFERENCES concepts(id),
  role TEXT NOT NULL CHECK(role IN('primary','form','meaning','function','context','error_source','curriculum')),
  provenance TEXT NOT NULL DEFAULT 'curated' CHECK(provenance IN('curated','external','inferred')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), PRIMARY KEY(weakness_id,concept_id,role)
);
CREATE UNIQUE INDEX IF NOT EXISTS weakness_one_primary_concept_idx ON weakness_concepts(weakness_id) WHERE role='primary';
CREATE INDEX IF NOT EXISTS weakness_concepts_concept_idx ON weakness_concepts(concept_id,role,weakness_id);
CREATE TABLE IF NOT EXISTS weakness_relations (
  subject_id INTEGER NOT NULL REFERENCES weaknesses(id), predicate TEXT NOT NULL CHECK(predicate IN('confusable_with','variant_of','supersedes','practice_together')),
  object_id INTEGER NOT NULL REFERENCES weaknesses(id), notes TEXT, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY(subject_id,predicate,object_id), CHECK(subject_id<>object_id)
);
CREATE INDEX IF NOT EXISTS weakness_relations_object_idx ON weakness_relations(object_id,predicate,subject_id);
CREATE TABLE IF NOT EXISTS collections (
  id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE, label TEXT NOT NULL,
  collection_type TEXT NOT NULL CHECK(collection_type IN('goal','curriculum','topic','campaign')),
  description TEXT, active INTEGER NOT NULL DEFAULT 1 CHECK(active IN(0,1)),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS collection_members (
  collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id), position INTEGER, notes TEXT,
  PRIMARY KEY(collection_id,weakness_id)
);
CREATE TABLE IF NOT EXISTS curriculum_mappings (
  id INTEGER PRIMARY KEY, weakness_id INTEGER REFERENCES weaknesses(id), concept_id INTEGER REFERENCES concepts(id),
  framework TEXT NOT NULL CHECK(framework IN('PCIC','CEFR','UD','RAE')), external_key TEXT NOT NULL, external_uri TEXT,
  relation TEXT NOT NULL CHECK(relation IN('exact','close','broader','narrower','related')), level_min TEXT, level_max TEXT, notes TEXT,
  CHECK((weakness_id IS NULL)<>(concept_id IS NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS curriculum_mapping_weakness_idx ON curriculum_mappings(weakness_id,framework,external_key) WHERE weakness_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS curriculum_mapping_concept_idx ON curriculum_mappings(concept_id,framework,external_key) WHERE concept_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS concept_drill_recommendations (
  concept_id INTEGER NOT NULL REFERENCES concepts(id),
  drill_type TEXT NOT NULL CHECK(drill_type IN('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
  stage TEXT NOT NULL CHECK(stage IN('recognition','controlled','transfer','fluency')), weight REAL NOT NULL CHECK(weight>0 AND weight<=1),
  PRIMARY KEY(concept_id,drill_type,stage)
);
CREATE TABLE IF NOT EXISTS weakness_drill_overrides (
  weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
  drill_type TEXT NOT NULL CHECK(drill_type IN('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
  stage TEXT NOT NULL CHECK(stage IN('recognition','controlled','transfer','fluency')), weight REAL NOT NULL CHECK(weight>=0 AND weight<=1),
  PRIMARY KEY(weakness_id,drill_type,stage)
);
