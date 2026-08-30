//! Ordered SQLite migrations.  The schema-3 snapshot is deliberately handled
//! before the latest-schema DDL is considered, so opening a database can never
//! turn a partially migrated database into an apparently current one.

use crate::{fsrs_adapter, taxonomy};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

const LATEST_VERSION: i64 = 7;
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (3, "003_baseline_snapshot", "aprendiendo-schema-3-snapshot"),
    (4, "004_taxonomy_graph", "aprendiendo-taxonomy-graph-v1"),
    (
        5,
        "005_evidence_integrity",
        "aprendiendo-evidence-integrity-v1",
    ),
    (6, "006_scheduler_audit", "aprendiendo-scheduler-audit-v1"),
    (
        7,
        "007_operational_cleanup",
        "aprendiendo-operational-cleanup-v1",
    ),
];

pub fn run(c: &mut Connection) -> Result<()> {
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=10000; PRAGMA journal_mode=WAL;")?;
    if !has_any_user_tables(c)? {
        initialize_latest(c)?;
        return postflight(c);
    }

    if !table_exists(c, "schema_migrations")? {
        if is_exact_schema3_snapshot(c)? {
            let tx = c.transaction()?;
            tx.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, checksum TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')))" )?;
            record_version(&tx, 3)?;
            tx.commit()?;
        } else if table_exists(c, "weaknesses")? {
            // This compatibility path is only for tiny pre-FSRS development
            // catalogs. It has no historical data to classify or schedule.
            initialize_legacy_catalog(c)?;
            return postflight(c);
        } else {
            bail!("database is neither empty nor the asserted schema-3 snapshot");
        }
    }

    validate_migration_registry(c)?;
    for &(version, _, _) in MIGRATIONS {
        if version == 3 {
            continue;
        }
        if !migration_applied(c, version)? {
            apply_migration(c, version)?;
        }
    }
    postflight(c)
}

fn initialize_latest(c: &mut Connection) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute_batch(include_str!("../sql/sqlite_schema.sql"))?;
    seed_exercise_types(&tx)?;
    seed_taxonomy(&tx)?;
    insert_default_scheduler(&tx)?;
    for &(version, _, _) in MIGRATIONS {
        record_version(&tx, version)?;
    }
    tx.execute(
        "INSERT INTO schema_meta(key,value) VALUES('schema_version','7')
         ON CONFLICT(key) DO UPDATE SET value='7'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn initialize_legacy_catalog(c: &mut Connection) -> Result<()> {
    let tx = c.transaction()?;
    add_legacy_columns(&tx)?;
    tx.execute_batch(include_str!("../sql/sqlite_schema.sql"))?;
    seed_exercise_types(&tx)?;
    seed_taxonomy(&tx)?;
    insert_default_scheduler(&tx)?;
    for &(version, _, _) in MIGRATIONS {
        record_version(&tx, version)?;
    }
    tx.execute(
        "INSERT INTO schema_meta(key,value) VALUES('schema_version','7')
         ON CONFLICT(key) DO UPDATE SET value='7'",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

fn add_legacy_columns(tx: &Transaction<'_>) -> Result<()> {
    let additions = [
        (
            "due_date",
            "ALTER TABLE weaknesses ADD COLUMN due_date TEXT",
        ),
        (
            "interval_days",
            "ALTER TABLE weaknesses ADD COLUMN interval_days INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "ease_factor",
            "ALTER TABLE weaknesses ADD COLUMN ease_factor REAL NOT NULL DEFAULT 2.5",
        ),
        (
            "repetitions",
            "ALTER TABLE weaknesses ADD COLUMN repetitions INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "lapses",
            "ALTER TABLE weaknesses ADD COLUMN lapses INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "last_reviewed",
            "ALTER TABLE weaknesses ADD COLUMN last_reviewed TEXT",
        ),
        (
            "recommended_drill_types",
            "ALTER TABLE weaknesses ADD COLUMN recommended_drill_types TEXT",
        ),
        (
            "fsrs_stability",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_stability REAL",
        ),
        (
            "fsrs_difficulty",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_difficulty REAL",
        ),
        (
            "fsrs_due_at",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_due_at TEXT",
        ),
        (
            "fsrs_last_review_at",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_last_review_at TEXT",
        ),
        (
            "fsrs_algorithm_version",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_algorithm_version TEXT",
        ),
        (
            "fsrs_parameters_version",
            "ALTER TABLE weaknesses ADD COLUMN fsrs_parameters_version INTEGER",
        ),
    ];
    for (column, sql) in additions {
        if !column_exists(tx, "weaknesses", column)? {
            tx.execute(sql, [])?;
        }
    }
    Ok(())
}

fn apply_migration(c: &mut Connection, version: i64) -> Result<()> {
    let tx = c.transaction()?;
    match version {
        4 => migration_taxonomy(&tx)?,
        5 => migration_evidence(&tx)?,
        6 => migration_scheduler(&tx)?,
        7 => migration_cleanup(&tx)?,
        _ => bail!("unsupported migration version {version}"),
    }
    record_version(&tx, version)?;
    tx.commit()?;
    postflight_at(c, version).with_context(|| format!("postflight after migration {version}"))
}

fn migration_taxonomy(tx: &Transaction<'_>) -> Result<()> {
    for (column, sql) in [
        (
            "target_type",
            "ALTER TABLE weaknesses ADD COLUMN target_type TEXT NOT NULL DEFAULT 'grammatical_construction' CHECK (target_type IN ('grammatical_construction','lexical_item','lexical_chunk','form_meaning_contrast','pronunciation','orthography','discourse_strategy','sociopragmatic_choice'))",
        ),
        (
            "target_status",
            "ALTER TABLE weaknesses ADD COLUMN target_status TEXT NOT NULL DEFAULT 'active' CHECK (target_status IN ('candidate','active','suspended','retired','merged'))",
        ),
        (
            "classification_pending",
            "ALTER TABLE weaknesses ADD COLUMN classification_pending INTEGER NOT NULL DEFAULT 0 CHECK (classification_pending IN (0,1))",
        ),
    ] {
        if !column_exists(tx, "weaknesses", column)? {
            tx.execute(sql, [])?;
        }
    }
    tx.execute_batch(TAXONOMY_DDL)?;
    seed_taxonomy(tx)
}

fn migration_evidence(tx: &Transaction<'_>) -> Result<()> {
    // All current item rows are empty, but this rebuild preserves them if a
    // future schema-3 copy contains item data and adds the required checks.
    tx.execute_batch(
        "CREATE TABLE practice_items_new (
          id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          item_no INTEGER NOT NULL CHECK (item_no >= 1),
          drill_type TEXT NOT NULL CHECK (drill_type IN ('translation','situational_response','sentence_transformation','question_answer','sentence_completion','sentence_combining','error_correction','minimal_pair_choice','dialogue_completion','micro_story','retell','fluency_4_3_2')),
          prompt TEXT NOT NULL, prompt_fingerprint TEXT, response TEXT, corrected_response TEXT, reference_answer TEXT, feedback TEXT,
          outcome TEXT CHECK (outcome IS NULL OR outcome IN ('incorrect','partially_correct','correct','omitted')),
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(id,session_id), UNIQUE(session_id,item_no)
        );
        INSERT INTO practice_items_new(id,session_id,item_no,drill_type,prompt,response,corrected_response,reference_answer,feedback,outcome,created_at)
          SELECT id,session_id,item_no,drill_type,prompt,response,corrected_response,reference_answer,feedback,outcome,created_at FROM practice_items;
        CREATE TABLE practice_item_targets_new (
          practice_item_id INTEGER NOT NULL REFERENCES practice_items_new(id) ON DELETE CASCADE,
          weakness_id INTEGER NOT NULL REFERENCES weaknesses(id), PRIMARY KEY(practice_item_id,weakness_id)
        );
        INSERT INTO practice_item_targets_new SELECT practice_item_id,weakness_id FROM practice_item_targets;
        CREATE TABLE attempts_new (
          id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1), practice_item_id INTEGER, transcript TEXT NOT NULL,
          target_duration_seconds INTEGER CHECK (target_duration_seconds IS NULL OR target_duration_seconds > 0),
          actual_duration_milliseconds INTEGER CHECK (actual_duration_milliseconds IS NULL OR actual_duration_milliseconds > 0),
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(id,session_id), UNIQUE(session_id,attempt_no),
          FOREIGN KEY(practice_item_id,session_id) REFERENCES practice_items_new(id,session_id) ON DELETE CASCADE
        );
        INSERT INTO attempts_new(id,session_id,attempt_no,practice_item_id,transcript,target_duration_seconds,created_at)
          SELECT id,session_id,attempt_no,practice_item_id,transcript,target_duration_seconds,created_at FROM attempts;
        CREATE TABLE observations_new (
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
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(session_id,observation_no),
          FOREIGN KEY(attempt_id,session_id) REFERENCES attempts_new(id,session_id) ON DELETE CASCADE,
          FOREIGN KEY(practice_item_id,session_id) REFERENCES practice_items_new(id,session_id) ON DELETE CASCADE
        );
        INSERT INTO observations_new(id,session_id,attempt_id,practice_item_id,weakness_id,observation_no,outcome,role,assessment_phase,hint_level,learner_effort,evidence_source,evidence_strength,severity,produced,correction,error_span,notes,created_at)
          SELECT id,session_id,attempt_id,practice_item_id,weakness_id,
                 row_number() OVER (PARTITION BY session_id ORDER BY id),outcome,COALESCE(role,'incidental'),'historical','none',NULL,'legacy',evidence_strength,severity,produced,correction,error_span,notes,created_at
          FROM observations;
        DROP TABLE observations;
        DROP TABLE attempts;
        DROP TABLE practice_item_targets;
        DROP TABLE practice_items;
        ALTER TABLE practice_items_new RENAME TO practice_items;
        ALTER TABLE practice_item_targets_new RENAME TO practice_item_targets;
        ALTER TABLE attempts_new RENAME TO attempts;
        ALTER TABLE observations_new RENAME TO observations;"
    )?;
    normalize_timestamps(tx)?;
    Ok(())
}

fn migration_scheduler(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE scheduler_parameter_sets (
          id INTEGER PRIMARY KEY, algorithm TEXT NOT NULL, algorithm_version TEXT NOT NULL,
          parameters_json TEXT NOT NULL CHECK (json_valid(parameters_json)), source TEXT NOT NULL CHECK (source IN ('default','optimized')),
          training_summary_json TEXT CHECK (training_summary_json IS NULL OR json_valid(training_summary_json)),
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          UNIQUE(algorithm,algorithm_version,parameters_json)
        );
        CREATE TABLE scheduler_items (
          id INTEGER PRIMARY KEY, weakness_id INTEGER NOT NULL REFERENCES weaknesses(id),
          track TEXT NOT NULL DEFAULT 'general' CHECK (track IN ('general','recognition','cued_production','controlled_production','spontaneous_production')),
          stability REAL, difficulty REAL, due_learning_day TEXT, last_review_at TEXT, algorithm_version TEXT,
          parameter_set_id INTEGER REFERENCES scheduler_parameter_sets(id), active INTEGER NOT NULL DEFAULT 1 CHECK(active IN(0,1)),
          created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(weakness_id,track),
          CHECK ((stability IS NULL AND difficulty IS NULL AND due_learning_day IS NULL AND last_review_at IS NULL AND algorithm_version IS NULL AND parameter_set_id IS NULL) OR (stability IS NOT NULL AND difficulty IS NOT NULL AND due_learning_day IS NOT NULL AND last_review_at IS NOT NULL AND algorithm_version IS NOT NULL AND parameter_set_id IS NOT NULL))
        );
        CREATE INDEX scheduler_items_queue_idx ON scheduler_items(active,due_learning_day);"
    )?;
    let old: (String, String, f64, String, i64, String, String) = tx.query_row(
        "SELECT algorithm,algorithm_version,desired_retention,timezone,day_cutoff_hour,parameters_json,parameters_source FROM scheduler_config WHERE id=1",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
    tx.execute("INSERT INTO scheduler_parameter_sets(id,algorithm,algorithm_version,parameters_json,source) VALUES(1,?1,?2,?3,?4)", params![old.0,old.1,old.5,old.6])?;
    tx.execute("INSERT INTO scheduler_items(weakness_id,track,active) SELECT id,'general',active FROM weaknesses", [])?;
    tx.execute_batch(
        "CREATE TABLE scheduler_config_new (
          id INTEGER PRIMARY KEY CHECK(id=1), active_parameter_set_id INTEGER NOT NULL REFERENCES scheduler_parameter_sets(id),
          desired_retention REAL NOT NULL CHECK(desired_retention BETWEEN 0.70 AND 0.97), timezone TEXT NOT NULL DEFAULT 'Europe/Madrid',
          day_cutoff_hour INTEGER NOT NULL DEFAULT 4 CHECK(day_cutoff_hour BETWEEN 0 AND 23),
          updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
        );
        INSERT INTO scheduler_config_new(id,active_parameter_set_id,desired_retention,timezone,day_cutoff_hour,updated_at)
          SELECT 1,1,desired_retention,timezone,day_cutoff_hour,updated_at FROM scheduler_config;
        DROP TABLE scheduler_config;
        ALTER TABLE scheduler_config_new RENAME TO scheduler_config;
        CREATE TABLE weakness_reviews_new (
          id INTEGER PRIMARY KEY, scheduler_item_id INTEGER NOT NULL REFERENCES scheduler_items(id), session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
          reviewed_at TEXT NOT NULL, review_learning_day TEXT NOT NULL, rating INTEGER NOT NULL CHECK(rating BETWEEN 1 AND 4),
          rating_source TEXT NOT NULL CHECK(rating_source IN ('learner','assistant_suggested_confirmed','assistant_suggested')), rating_rationale TEXT,
          retrieval_mode TEXT NOT NULL CHECK(retrieval_mode IN ('recognition','cued_production','controlled_production','spontaneous_production')),
          evidence_strength TEXT NOT NULL CHECK(evidence_strength IN ('recognition','cued_production','controlled_production','spontaneous_production')),
          evidence_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(evidence_json)), elapsed_days INTEGER NOT NULL CHECK(elapsed_days>=0), desired_retention REAL NOT NULL CHECK(desired_retention BETWEEN 0.70 AND 0.97),
          retrievability_before REAL CHECK(retrievability_before IS NULL OR (retrievability_before >= 0 AND retrievability_before <= 1)),
          stability_before REAL CHECK(stability_before IS NULL OR stability_before > 0),
          difficulty_before REAL CHECK(difficulty_before IS NULL OR (difficulty_before >= 1 AND difficulty_before <= 10)),
          stability_after REAL NOT NULL CHECK(stability_after > 0), difficulty_after REAL NOT NULL CHECK(difficulty_after >= 1 AND difficulty_after <= 10),
          scheduled_interval_days INTEGER NOT NULL CHECK(scheduled_interval_days>=1), due_learning_day TEXT NOT NULL, algorithm TEXT NOT NULL, algorithm_version TEXT NOT NULL,
          parameter_set_id INTEGER NOT NULL REFERENCES scheduler_parameter_sets(id), created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), UNIQUE(session_id,scheduler_item_id)
        );
        INSERT INTO weakness_reviews_new SELECT id,
          (SELECT id FROM scheduler_items WHERE weakness_id=weakness_reviews.weakness_id AND track='general'),session_id,reviewed_at,
          due_at,rating,'assistant_suggested',NULL,retrieval_mode,evidence_strength,evidence_json,elapsed_days,desired_retention,retrievability_before,stability_before,difficulty_before,stability_after,difficulty_after,scheduled_interval_days,due_at,algorithm,algorithm_version,1,created_at
          FROM weakness_reviews;
        DROP TABLE weakness_reviews;
        ALTER TABLE weakness_reviews_new RENAME TO weakness_reviews;
        CREATE INDEX weakness_reviews_item_time_idx ON weakness_reviews(scheduler_item_id,reviewed_at);
        CREATE TABLE review_observations (
          review_id INTEGER NOT NULL REFERENCES weakness_reviews(id) ON DELETE CASCADE, observation_id INTEGER NOT NULL REFERENCES observations(id),
          evidence_role TEXT NOT NULL CHECK(evidence_role IN ('initial','supporting','contradicting','transfer')), PRIMARY KEY(review_id,observation_id)
        );
        CREATE INDEX review_observations_observation_idx ON review_observations(observation_id,review_id);"
    )?;
    Ok(())
}

fn migration_cleanup(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE exercise_types (key TEXT PRIMARY KEY,label TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1 CHECK(active IN(0,1)));
         INSERT INTO exercise_types(key,label) VALUES
          ('fluency_4_3_2','4-3-2'),('production_drill','production drill'),('translation_drill','translation drill'),
          ('dele_a2_oral_microdrill','DELE A2 oral microdrill'),('agreement_disagreement_drill','agreement/disagreement drill'),('guided_conversation','guided conversation');"
    )?;
    tx.execute(
        "ALTER TABLE sessions ADD COLUMN exercise_type_key TEXT REFERENCES exercise_types(key)",
        [],
    )?;
    for (old, key) in [
        ("4-3-2", "fluency_4_3_2"),
        ("production drill", "production_drill"),
        ("translation_drill", "translation_drill"),
        ("DELE A2 oral microdrill", "dele_a2_oral_microdrill"),
        (
            "agreement/disagreement drill",
            "agreement_disagreement_drill",
        ),
        ("guided conversation drill", "guided_conversation"),
    ] {
        tx.execute(
            "UPDATE sessions SET exercise_type_key=?1 WHERE exercise_type=?2",
            params![key, old],
        )?;
    }
    let unknown: i64 = tx.query_row(
        "SELECT count(*) FROM sessions WHERE exercise_type_key IS NULL",
        [],
        |r| r.get(0),
    )?;
    if unknown != 0 {
        bail!("snapshot contains {unknown} unknown exercise types");
    }

    tx.execute_batch(
        "CREATE TABLE recorded_requests_new (
          idempotency_key TEXT PRIMARY KEY, session_id INTEGER NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
          request_hash TEXT NOT NULL, response_json TEXT NOT NULL CHECK(json_valid(response_json) AND json_type(response_json)='object'),
          replayable INTEGER NOT NULL CHECK(replayable IN(0,1)), created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
          CHECK(replayable=0 OR (length(request_hash)=64 AND request_hash NOT GLOB '*[^0-9a-f]*'))
        );
        INSERT INTO recorded_requests_new(idempotency_key,session_id,request_hash,response_json,replayable,created_at)
          SELECT idempotency_key,session_id,request_hash,response_json,CASE WHEN request_hash='' OR response_json='{}' THEN 0 ELSE 1 END,created_at FROM recorded_requests;
        DROP TABLE recorded_requests;
        ALTER TABLE recorded_requests_new RENAME TO recorded_requests;
        CREATE INDEX IF NOT EXISTS prompt_fingerprint_created_idx ON practice_items(prompt_fingerprint,created_at);"
    )?;
    Ok(())
}

fn normalize_timestamps(tx: &Transaction<'_>) -> Result<()> {
    for table in ["sessions", "attempts", "observations"] {
        let mut stmt = tx.prepare(&format!("SELECT id,created_at FROM {table}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (id, value) in rows {
            tx.execute(
                &format!("UPDATE {table} SET created_at=?1 WHERE id=?2"),
                params![canonical_timestamp(&value)?, id],
            )?;
        }
    }
    Ok(())
}

fn canonical_timestamp(value: &str) -> Result<String> {
    let normalized = value.replace(' ', "T").replace("+00", "Z");
    let dt = DateTime::parse_from_rfc3339(&normalized)
        .map_err(|e| anyhow!("invalid legacy timestamp {value:?}: {e}"))?
        .with_timezone(&Utc);
    Ok(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn seed_exercise_types(c: &Connection) -> Result<()> {
    for (key, label) in [
        ("fluency_4_3_2", "4-3-2"),
        ("production_drill", "production drill"),
        ("translation_drill", "translation drill"),
        ("dele_a2_oral_microdrill", "DELE A2 oral microdrill"),
        (
            "agreement_disagreement_drill",
            "agreement/disagreement drill",
        ),
        ("guided_conversation", "guided conversation"),
    ] {
        c.execute(
            "INSERT OR IGNORE INTO exercise_types(key,label) VALUES(?1,?2)",
            params![key, label],
        )?;
    }
    Ok(())
}

fn insert_default_scheduler(c: &Connection) -> Result<()> {
    c.execute("INSERT OR IGNORE INTO scheduler_parameter_sets(id,algorithm,algorithm_version,parameters_json,source) VALUES(1,'fsrs','FSRS-6',?1,'default')", [fsrs_adapter::default_parameters_json()])?;
    c.execute("INSERT OR IGNORE INTO scheduler_config(id,active_parameter_set_id,desired_retention,timezone,day_cutoff_hour) VALUES(1,1,0.90,'Europe/Madrid',4)", [])?;
    c.execute("INSERT OR IGNORE INTO scheduler_items(weakness_id,track,active) SELECT id,'general',active FROM weaknesses", [])?;
    Ok(())
}

fn migration_applied(c: &Connection, version: i64) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version=?1)",
        [version],
        |r| r.get(0),
    )?)
}

fn record_version(c: &Connection, version: i64) -> Result<()> {
    let (_, name, checksum) = MIGRATIONS
        .iter()
        .find(|x| x.0 == version)
        .ok_or_else(|| anyhow!("unknown migration {version}"))?;
    c.execute(
        "INSERT INTO schema_migrations(version,name,checksum) VALUES(?1,?2,?3)",
        params![version, name, checksum],
    )?;
    Ok(())
}

fn validate_migration_registry(c: &Connection) -> Result<()> {
    let mut stmt =
        c.prepare("SELECT version,name,checksum FROM schema_migrations ORDER BY version")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (version, name, checksum) in rows {
        let expected = MIGRATIONS
            .iter()
            .find(|x| x.0 == version)
            .ok_or_else(|| anyhow!("unknown applied migration version {version}"))?;
        if name != expected.1 || checksum != expected.2 {
            bail!("checksum mismatch for migration {version}");
        }
    }
    Ok(())
}

fn postflight(c: &Connection) -> Result<()> {
    postflight_at(c, LATEST_VERSION)
}

fn postflight_at(c: &Connection, expected_version: i64) -> Result<()> {
    let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        bail!("SQLite integrity_check failed: {integrity}");
    }
    let mut fk = c.prepare("PRAGMA foreign_key_check")?;
    if fk.query([])?.next()?.is_some() {
        bail!("SQLite foreign_key_check failed");
    }
    let applied: i64 = c.query_row("SELECT max(version) FROM schema_migrations", [], |r| {
        r.get(0)
    })?;
    if applied != expected_version {
        bail!("database migration version is {applied}, expected {expected_version}");
    }
    taxonomy::validate_primary_links(c)?;
    Ok(())
}

fn has_any_user_tables(c: &Connection) -> Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')", [], |r| r.get(0))?)
}
fn table_exists(c: &Connection, table: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )?)
}
fn column_exists(c: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = c.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .any(|v| v.as_deref() == Ok(column)))
}

fn is_exact_schema3_snapshot(c: &Connection) -> Result<bool> {
    if !table_exists(c, "schema_meta")? {
        return Ok(false);
    }
    let version: Option<String> = c
        .query_row(
            "SELECT value FROM schema_meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if version.as_deref() != Some("3") {
        return Ok(false);
    }
    for (table, expected) in [
        ("sessions", 17),
        ("attempts", 24),
        ("observations", 162),
        ("weaknesses", 39),
        ("practice_items", 0),
        ("practice_item_targets", 0),
        ("weakness_reviews", 0),
        ("scheduler_config", 1),
        ("recorded_requests", 2),
        ("schema_meta", 1),
    ] {
        let count: i64 = c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        if count != expected {
            bail!(
                "schema-3 snapshot preflight failed: {table} has {count} rows, expected {expected}"
            );
        }
    }
    let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        bail!("schema-3 snapshot integrity_check failed: {integrity}");
    }
    if c.prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        bail!("schema-3 snapshot has foreign-key violations");
    }
    let dates: (i64, String, String) = c.query_row(
        "SELECT count(DISTINCT session_date),min(session_date),max(session_date) FROM sessions",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if dates != (11, "2026-08-02".into(), "2026-08-28".into()) {
        bail!("schema-3 snapshot session dates do not match the asserted source");
    }
    let outcomes: (i64, i64) = c.query_row(
        "SELECT sum(outcome='incorrect'),sum(outcome='correct') FROM observations",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if outcomes != (82, 80) {
        bail!("schema-3 snapshot outcome counts do not match the asserted source");
    }
    let links: (i64,i64) = c.query_row("SELECT sum(attempt_id IS NOT NULL),sum(attempt_id IS NULL AND practice_item_id IS NULL) FROM observations", [], |r| Ok((r.get(0)?,r.get(1)?)))?;
    if links != (131, 31) {
        bail!("schema-3 snapshot observation links do not match the asserted source");
    }
    let text_counts: (i64,i64,i64) = c.query_row("SELECT sum(produced IS NOT NULL),sum(correction IS NOT NULL),sum(notes IS NOT NULL) FROM observations", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if text_counts != (162, 142, 153) {
        bail!("schema-3 snapshot observation text counts do not match the asserted source");
    }
    let session_types: i64 = c.query_row(
        "SELECT count(*) FROM sessions WHERE exercise_type='4-3-2'",
        [],
        |r| r.get(0),
    )?;
    if session_types != 5 {
        bail!("schema-3 snapshot must contain five 4-3-2 sessions");
    }
    let not_incidental: i64 = c.query_row(
        "SELECT count(*) FROM observations WHERE role <> 'incidental'",
        [],
        |r| r.get(0),
    )?;
    let evidence: i64 = c.query_row("SELECT count(*) FROM observations WHERE evidence_strength IS NOT NULL OR severity IS NOT NULL", [], |r| r.get(0))?;
    if not_incidental != 0 || evidence != 0 {
        bail!("schema-3 snapshot contains non-historical evidence fields");
    }
    let no_review_data: i64 = c.query_row("SELECT count(*) FROM weaknesses WHERE fsrs_stability IS NOT NULL OR fsrs_difficulty IS NOT NULL OR fsrs_due_at IS NOT NULL OR fsrs_last_review_at IS NOT NULL OR fsrs_algorithm_version IS NOT NULL OR fsrs_parameters_version IS NOT NULL", [], |r| r.get(0))?;
    if no_review_data != 0 {
        bail!("schema-3 snapshot contains initialized FSRS state");
    }
    let bad_requests: i64 = c.query_row(
        "SELECT count(*) FROM recorded_requests WHERE request_hash <> '' OR response_json <> '{}'",
        [],
        |r| r.get(0),
    )?;
    if bad_requests != 0 {
        bail!("schema-3 snapshot idempotency rows are not the asserted legacy records");
    }
    let legacy_timestamps: (i64,i64,i64) = c.query_row("SELECT sum(created_at LIKE '%+00'),(SELECT sum(created_at LIKE '%+00') FROM attempts),(SELECT sum(created_at LIKE '%+00') FROM observations) FROM sessions", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    if legacy_timestamps != (15, 22, 135) {
        bail!("schema-3 snapshot timestamp counts do not match the asserted source");
    }
    let cross_session: i64 = c.query_row("SELECT count(*) FROM observations o JOIN attempts a ON a.id=o.attempt_id WHERE a.session_id<>o.session_id", [], |r| r.get(0))?;
    if cross_session != 0 {
        bail!("schema-3 snapshot contains a cross-session observation");
    }
    Ok(true)
}

const TAXONOMY_DDL: &str = include_str!("../sql/taxonomy_migration.sql");

const SCHEMES: &[(&str, &str)] = &[
    ("linguistic_form", "Linguistic form"),
    ("communicative_use", "Communicative use"),
    ("meaning_context", "Meaning and context"),
];
const CONCEPTS: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "form.unclassified",
        "linguistic_form",
        "Unclassified linguistic form",
        None,
    ),
    ("form.grammar", "linguistic_form", "Grammar", None),
    (
        "form.grammar.morphology",
        "linguistic_form",
        "Morphology",
        Some("form.grammar"),
    ),
    (
        "form.grammar.morphology.agreement",
        "linguistic_form",
        "Agreement",
        Some("form.grammar.morphology"),
    ),
    (
        "form.grammar.morphology.invariability",
        "linguistic_form",
        "Invariable modifiers",
        Some("form.grammar.morphology"),
    ),
    (
        "form.grammar.verb_system",
        "linguistic_form",
        "Verb system",
        Some("form.grammar"),
    ),
    (
        "form.grammar.verb_system.tense_aspect",
        "linguistic_form",
        "Tense and aspect",
        Some("form.grammar.verb_system"),
    ),
    (
        "form.grammar.verb_system.mood",
        "linguistic_form",
        "Mood and modality",
        Some("form.grammar.verb_system"),
    ),
    (
        "form.grammar.verb_system.mood.subjunctive",
        "linguistic_form",
        "Subjunctive mood",
        Some("form.grammar.verb_system.mood"),
    ),
    (
        "form.grammar.verb_system.periphrasis",
        "linguistic_form",
        "Verbal periphrases",
        Some("form.grammar.verb_system"),
    ),
    (
        "form.grammar.syntax",
        "linguistic_form",
        "Syntax",
        Some("form.grammar"),
    ),
    (
        "form.grammar.syntax.clitics",
        "linguistic_form",
        "Object and reflexive clitics",
        Some("form.grammar.syntax"),
    ),
    (
        "form.grammar.syntax.valency",
        "linguistic_form",
        "Valency and argument structure",
        Some("form.grammar.syntax"),
    ),
    (
        "form.grammar.syntax.subordination",
        "linguistic_form",
        "Subordination",
        Some("form.grammar.syntax"),
    ),
    (
        "form.grammar.syntax.prepositions",
        "linguistic_form",
        "Prepositional selection",
        Some("form.grammar.syntax"),
    ),
    (
        "form.grammar.syntax.copular",
        "linguistic_form",
        "Copular constructions",
        Some("form.grammar.syntax"),
    ),
    (
        "form.lexis",
        "linguistic_form",
        "Lexis and phraseology",
        None,
    ),
    (
        "form.lexis.lexeme_choice",
        "linguistic_form",
        "Lexeme choice",
        Some("form.lexis"),
    ),
    (
        "form.lexis.collocation",
        "linguistic_form",
        "Collocations",
        Some("form.lexis"),
    ),
    (
        "form.lexis.fixed_expression",
        "linguistic_form",
        "Fixed expressions",
        Some("form.lexis"),
    ),
    (
        "form.lexis.lexical_grammar",
        "linguistic_form",
        "Lexically governed grammar",
        Some("form.lexis"),
    ),
    (
        "form.pronunciation",
        "linguistic_form",
        "Pronunciation and prosody",
        None,
    ),
    ("form.orthography", "linguistic_form", "Orthography", None),
    (
        "use.unclassified",
        "communicative_use",
        "Unclassified communicative use",
        None,
    ),
    (
        "use.describe_narrate_past",
        "communicative_use",
        "Describe and narrate in the past",
        None,
    ),
    (
        "use.evaluate_completed_event",
        "communicative_use",
        "Evaluate a completed event",
        None,
    ),
    (
        "use.report_influence",
        "communicative_use",
        "Report commands and influence",
        None,
    ),
    (
        "use.express_purpose",
        "communicative_use",
        "Express purpose",
        None,
    ),
    (
        "use.express_duration",
        "communicative_use",
        "Express duration",
        None,
    ),
    (
        "use.express_cause",
        "communicative_use",
        "Express cause",
        None,
    ),
    (
        "use.express_space",
        "communicative_use",
        "Express spatial relations",
        None,
    ),
    (
        "use.express_desire_politeness",
        "communicative_use",
        "Express desire and mitigation",
        None,
    ),
    (
        "use.social_interaction",
        "communicative_use",
        "Manage social interaction",
        None,
    ),
    (
        "use.discourse",
        "communicative_use",
        "Organize discourse",
        None,
    ),
    (
        "use.sociopragmatics",
        "communicative_use",
        "Register and sociopragmatic choice",
        None,
    ),
    (
        "meaning.unclassified",
        "meaning_context",
        "Unclassified meaning or context",
        None,
    ),
    (
        "meaning.time",
        "meaning_context",
        "Time and temporal sequence",
        None,
    ),
    ("meaning.duration", "meaning_context", "Duration", None),
    ("meaning.cause", "meaning_context", "Cause", None),
    ("meaning.purpose", "meaning_context", "Purpose", None),
    (
        "meaning.space",
        "meaning_context",
        "Space and location",
        None,
    ),
    (
        "meaning.emotion_reaction",
        "meaning_context",
        "Emotion and reaction",
        None,
    ),
    (
        "domain.health",
        "meaning_context",
        "Health and the body",
        None,
    ),
    (
        "domain.travel",
        "meaning_context",
        "Travel and transport",
        None,
    ),
    (
        "domain.books_reading",
        "meaning_context",
        "Books and reading",
        None,
    ),
    (
        "domain.social_relations",
        "meaning_context",
        "Social relationships",
        None,
    ),
];

type TargetSeed = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const TARGETS: &[TargetSeed] = &[
    (
        "adverb_invariable_modifier",
        "grammatical_construction",
        "form.grammar.morphology.invariability",
        &[("form", "form.grammar.morphology.agreement")],
    ),
    (
        "agreement_gender_number",
        "grammatical_construction",
        "form.grammar.morphology.agreement",
        &[("form", "form.grammar.syntax")],
    ),
    (
        "acompanar_direct_object",
        "grammatical_construction",
        "form.grammar.syntax.valency",
        &[
            ("form", "form.grammar.syntax.prepositions"),
            ("context", "domain.social_relations"),
        ],
    ),
    (
        "doler_gustar_structure",
        "grammatical_construction",
        "form.grammar.syntax.valency",
        &[
            ("form", "form.grammar.syntax.clitics"),
            ("context", "domain.health"),
        ],
    ),
    (
        "impedir_a_alguien",
        "grammatical_construction",
        "form.grammar.syntax.valency",
        &[
            ("form", "form.grammar.syntax.prepositions"),
            ("form", "form.grammar.syntax.clitics"),
        ],
    ),
    (
        "duration_llevar_gerund",
        "grammatical_construction",
        "form.grammar.verb_system.periphrasis",
        &[
            ("function", "use.express_duration"),
            ("meaning", "meaning.duration"),
        ],
    ),
    (
        "iba_a_infinitive",
        "grammatical_construction",
        "form.grammar.verb_system.periphrasis",
        &[
            ("form", "form.grammar.verb_system.tense_aspect"),
            ("meaning", "meaning.time"),
        ],
    ),
    (
        "aplicarse_clitics",
        "grammatical_construction",
        "form.grammar.syntax.clitics",
        &[
            ("form", "form.grammar.syntax.valency"),
            ("context", "domain.health"),
        ],
    ),
    (
        "decirselo_form",
        "grammatical_construction",
        "form.grammar.syntax.clitics",
        &[("form", "form.orthography")],
    ),
    (
        "double_object_clitics",
        "grammatical_construction",
        "form.grammar.syntax.clitics",
        &[("form", "form.grammar.syntax.valency")],
    ),
    (
        "hacer_pensar",
        "grammatical_construction",
        "form.grammar.syntax.clitics",
        &[("form", "form.grammar.syntax.valency")],
    ),
    (
        "prometer_infinitive_clitics",
        "grammatical_construction",
        "form.grammar.syntax.clitics",
        &[("form", "form.grammar.syntax.valency")],
    ),
    (
        "aprovechar_para",
        "lexical_chunk",
        "form.lexis.lexical_grammar",
        &[
            ("form", "form.lexis.collocation"),
            ("function", "use.express_purpose"),
            ("meaning", "meaning.purpose"),
        ],
    ),
    (
        "club_de_lectura",
        "lexical_chunk",
        "form.lexis.collocation",
        &[("context", "domain.books_reading")],
    ),
    (
        "cometer_error",
        "lexical_chunk",
        "form.lexis.collocation",
        &[],
    ),
    (
        "dar_estres_energia",
        "lexical_chunk",
        "form.lexis.collocation",
        &[("meaning", "meaning.emotion_reaction")],
    ),
    (
        "faltar_tiempo",
        "lexical_chunk",
        "form.lexis.collocation",
        &[
            ("meaning", "meaning.time"),
            ("function", "use.express_duration"),
        ],
    ),
    (
        "hinchazon_disminuir",
        "lexical_chunk",
        "form.lexis.collocation",
        &[
            ("form", "form.lexis.lexeme_choice"),
            ("context", "domain.health"),
        ],
    ),
    (
        "reservar_asientos",
        "lexical_chunk",
        "form.lexis.collocation",
        &[
            ("form", "form.lexis.lexeme_choice"),
            ("context", "domain.travel"),
        ],
    ),
    (
        "cuanto_antes",
        "lexical_chunk",
        "form.lexis.fixed_expression",
        &[("meaning", "meaning.time")],
    ),
    (
        "durante_todo_periodo",
        "lexical_chunk",
        "form.lexis.fixed_expression",
        &[
            ("meaning", "meaning.duration"),
            ("function", "use.express_duration"),
        ],
    ),
    (
        "tratar_de_topic",
        "grammatical_construction",
        "form.lexis.lexical_grammar",
        &[
            ("form", "form.grammar.syntax.valency"),
            ("context", "domain.books_reading"),
        ],
    ),
    (
        "body_part_prepositions",
        "grammatical_construction",
        "form.grammar.syntax.prepositions",
        &[("context", "domain.health"), ("meaning", "meaning.space")],
    ),
    (
        "caused_by",
        "grammatical_construction",
        "form.grammar.syntax.prepositions",
        &[
            ("function", "use.express_cause"),
            ("meaning", "meaning.cause"),
        ],
    ),
    (
        "despedirse_de",
        "grammatical_construction",
        "form.grammar.syntax.prepositions",
        &[
            ("form", "form.lexis.lexical_grammar"),
            ("function", "use.social_interaction"),
            ("context", "domain.social_relations"),
        ],
    ),
    (
        "salir_de_place_event",
        "grammatical_construction",
        "form.grammar.syntax.prepositions",
        &[("meaning", "meaning.space"), ("context", "domain.travel")],
    ),
    (
        "space_para_piernas",
        "grammatical_construction",
        "form.grammar.syntax.prepositions",
        &[
            ("function", "use.express_space"),
            ("meaning", "meaning.space"),
            ("context", "domain.travel"),
        ],
    ),
    (
        "ser_estar_event_result",
        "form_meaning_contrast",
        "form.grammar.syntax.copular",
        &[("function", "use.evaluate_completed_event")],
    ),
    (
        "antes_de_que_subjunctive",
        "grammatical_construction",
        "form.grammar.syntax.subordination",
        &[
            ("form", "form.grammar.verb_system.mood.subjunctive"),
            ("meaning", "meaning.time"),
        ],
    ),
    (
        "para_que_subjunctive",
        "grammatical_construction",
        "form.grammar.syntax.subordination",
        &[
            ("form", "form.grammar.verb_system.mood.subjunctive"),
            ("function", "use.express_purpose"),
            ("meaning", "meaning.purpose"),
        ],
    ),
    (
        "past_nonspecific_relative_subjunctive",
        "grammatical_construction",
        "form.grammar.syntax.subordination",
        &[
            ("form", "form.grammar.verb_system.mood.subjunctive"),
            ("form", "form.grammar.verb_system.tense_aspect"),
        ],
    ),
    (
        "reported_command_past",
        "grammatical_construction",
        "form.grammar.syntax.subordination",
        &[
            ("form", "form.grammar.verb_system.mood.subjunctive"),
            ("function", "use.report_influence"),
        ],
    ),
    (
        "reported_influence_past",
        "grammatical_construction",
        "form.grammar.syntax.subordination",
        &[
            ("form", "form.grammar.verb_system.mood.subjunctive"),
            ("function", "use.report_influence"),
        ],
    ),
    (
        "preterite_ayer",
        "grammatical_construction",
        "form.grammar.verb_system.tense_aspect",
        &[
            ("meaning", "meaning.time"),
            ("function", "use.describe_narrate_past"),
        ],
    ),
    (
        "preterite_completed_evaluation",
        "grammatical_construction",
        "form.grammar.verb_system.tense_aspect",
        &[
            ("function", "use.evaluate_completed_event"),
            ("function", "use.describe_narrate_past"),
        ],
    ),
    (
        "preterite_imperfect_background",
        "form_meaning_contrast",
        "form.grammar.verb_system.tense_aspect",
        &[("function", "use.describe_narrate_past")],
    ),
    (
        "queria_querria_quisiera",
        "form_meaning_contrast",
        "form.grammar.verb_system.mood",
        &[
            ("form", "form.grammar.verb_system.tense_aspect"),
            ("function", "use.express_desire_politeness"),
            ("function", "use.sociopragmatics"),
        ],
    ),
    (
        "farmaceutico_vocab",
        "lexical_item",
        "form.lexis.lexeme_choice",
        &[("context", "domain.health")],
    ),
    (
        "paraguas_vocab",
        "lexical_item",
        "form.lexis.lexeme_choice",
        &[("context", "domain.travel")],
    ),
];

fn seed_taxonomy(c: &Connection) -> Result<()> {
    for (key, label) in SCHEMES {
        c.execute(
            "INSERT OR IGNORE INTO concept_schemes(key,label) VALUES(?1,?2)",
            params![key, label],
        )?;
    }
    for (key, scheme, label, parent) in CONCEPTS {
        c.execute("INSERT OR IGNORE INTO concepts(scheme_id,key,label) SELECT id,?1,?2 FROM concept_schemes WHERE key=?3", params![key,label,scheme])?;
        if let Some(parent) = parent {
            insert_concept_edge(c, key, "broader", parent, "curated", None)?;
        }
    }
    for (a, p, b) in [
        (
            "form.grammar.syntax.clitics",
            "requires",
            "form.grammar.syntax.valency",
        ),
        (
            "form.grammar.syntax.subordination",
            "requires",
            "form.grammar.verb_system.mood",
        ),
        (
            "form.grammar.verb_system.mood.subjunctive",
            "related",
            "form.grammar.syntax.subordination",
        ),
        (
            "use.evaluate_completed_event",
            "related",
            "use.describe_narrate_past",
        ),
        ("meaning.duration", "related", "meaning.time"),
    ] {
        insert_concept_edge(c, a, p, b, "curated", None)?;
    }
    let has_targets: bool =
        c.query_row("SELECT EXISTS(SELECT 1 FROM weaknesses)", [], |r| r.get(0))?;
    for (key, typ, primary, links) in TARGETS {
        if !has_targets {
            break;
        }
        let Some(wid) = c
            .query_row("SELECT id FROM weaknesses WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?
        else {
            bail!("taxonomy seed is missing weakness {key}");
        };
        c.execute("UPDATE weaknesses SET target_type=?1,target_status='active',classification_pending=0,active=1 WHERE id=?2", params![typ,wid])?;
        c.execute("DELETE FROM weakness_concepts WHERE weakness_id=?1", [wid])?;
        insert_target_link(c, wid, primary, "primary")?;
        for (role, concept) in *links {
            insert_target_link(c, wid, concept, role)?;
        }
    }
    for (a, p, b) in [
        (
            "reported_command_past",
            "practice_together",
            "reported_influence_past",
        ),
        (
            "preterite_completed_evaluation",
            "confusable_with",
            "preterite_imperfect_background",
        ),
        (
            "preterite_ayer",
            "practice_together",
            "preterite_imperfect_background",
        ),
        (
            "decirselo_form",
            "practice_together",
            "double_object_clitics",
        ),
        (
            "double_object_clitics",
            "practice_together",
            "prometer_infinitive_clitics",
        ),
        (
            "aplicarse_clitics",
            "practice_together",
            "double_object_clitics",
        ),
    ] {
        if has_targets {
            insert_target_relation(c, a, p, b, None)?;
        }
    }
    seed_drills(c)?;
    taxonomy::validate_primary_links(c)
}

fn insert_concept_edge(
    c: &Connection,
    a: &str,
    predicate: &str,
    b: &str,
    provenance: &str,
    notes: Option<&str>,
) -> Result<()> {
    taxonomy::validate_edge_predicate(predicate)?;
    let (a, b) = if matches!(predicate, "contrasts_with" | "related") {
        taxonomy::canonical_pair(a, b)
    } else {
        (a, b)
    };
    let aid = taxonomy::concept_id(c, a)?;
    let bid = taxonomy::concept_id(c, b)?;
    if matches!(predicate, "broader" | "requires") {
        taxonomy::validate_edge_does_not_cycle(c, aid, predicate, bid)?;
    }
    c.execute("INSERT OR IGNORE INTO concept_edges(subject_id,predicate,object_id,provenance,notes) VALUES(?1,?2,?3,?4,?5)", params![aid,predicate,bid,provenance,notes])?;
    Ok(())
}
fn insert_target_link(c: &Connection, wid: i64, concept: &str, role: &str) -> Result<()> {
    let cid = taxonomy::concept_id(c, concept)?;
    c.execute(
        "INSERT INTO weakness_concepts(weakness_id,concept_id,role) VALUES(?1,?2,?3)",
        params![wid, cid, role],
    )?;
    Ok(())
}
fn insert_target_relation(
    c: &Connection,
    a: &str,
    predicate: &str,
    b: &str,
    notes: Option<&str>,
) -> Result<()> {
    taxonomy::validate_target_relation_predicate(predicate)?;
    let (a, b) = if matches!(
        predicate,
        "confusable_with" | "variant_of" | "practice_together"
    ) {
        taxonomy::canonical_pair(a, b)
    } else {
        (a, b)
    };
    let aid: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [a], |r| r.get(0))?;
    let bid: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [b], |r| r.get(0))?;
    c.execute("INSERT OR IGNORE INTO weakness_relations(subject_id,predicate,object_id,notes) VALUES(?1,?2,?3,?4)",params![aid,predicate,bid,notes])?;
    Ok(())
}

fn seed_drills(c: &Connection) -> Result<()> {
    let grammar = [
        "form.grammar.morphology.agreement",
        "form.grammar.morphology.invariability",
        "form.grammar.verb_system.tense_aspect",
        "form.grammar.verb_system.mood",
        "form.grammar.verb_system.periphrasis",
        "form.grammar.syntax.clitics",
        "form.grammar.syntax.valency",
        "form.grammar.syntax.subordination",
        "form.grammar.syntax.prepositions",
        "form.grammar.syntax.copular",
    ];
    for key in grammar {
        let id = taxonomy::concept_id(c, key)?;
        for (d, s, w) in [
            ("sentence_transformation", "controlled", 1.0),
            ("sentence_completion", "controlled", 0.9),
            ("translation", "controlled", 0.7),
            ("situational_response", "transfer", 0.8),
        ] {
            c.execute("INSERT OR IGNORE INTO concept_drill_recommendations(concept_id,drill_type,stage,weight) VALUES(?1,?2,?3,?4)",params![id,d,s,w])?;
        }
    }
    let lexis = [
        "form.lexis.lexeme_choice",
        "form.lexis.collocation",
        "form.lexis.fixed_expression",
        "form.lexis.lexical_grammar",
    ];
    for key in lexis {
        let id = taxonomy::concept_id(c, key)?;
        for (d, s, w) in [
            ("sentence_completion", "controlled", 0.9),
            ("translation", "controlled", 0.7),
            ("dialogue_completion", "transfer", 0.9),
            ("micro_story", "transfer", 0.7),
        ] {
            c.execute("INSERT OR IGNORE INTO concept_drill_recommendations(concept_id,drill_type,stage,weight) VALUES(?1,?2,?3,?4)",params![id,d,s,w])?;
        }
    }
    let id = taxonomy::concept_id(c, "use.discourse")?;
    for (d, s, w) in [
        ("retell", "transfer", 1.0),
        ("micro_story", "transfer", 0.8),
        ("fluency_4_3_2", "fluency", 1.0),
    ] {
        c.execute("INSERT OR IGNORE INTO concept_drill_recommendations(concept_id,drill_type,stage,weight) VALUES(?1,?2,?3,?4)",params![id,d,s,w])?;
    }
    if !c.query_row("SELECT EXISTS(SELECT 1 FROM weaknesses)", [], |r| r.get(0))? {
        return Ok(());
    }
    for key in [
        "ser_estar_event_result",
        "preterite_imperfect_background",
        "queria_querria_quisiera",
    ] {
        let wid: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [key], |r| {
            r.get(0)
        })?;
        for (d, s, w) in [
            ("minimal_pair_choice", "recognition", 0.7),
            ("error_correction", "controlled", 0.9),
            ("situational_response", "transfer", 1.0),
        ] {
            c.execute("INSERT OR IGNORE INTO weakness_drill_overrides(weakness_id,drill_type,stage,weight) VALUES(?1,?2,?3,?4)",params![wid,d,s,w])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn empty_database_gets_latest_schema_and_seeds() {
        let path = std::env::temp_dir().join(format!(
            "aprendiendo-migration-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let mut c = Connection::open(&path).unwrap();
        run(&mut c).unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM concept_schemes", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            5
        );
        drop(c);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn exact_live_snapshot_migrates_without_inferred_reviews() {
        let source =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/aprendiendo-live.sqlite3");
        if !source.exists() {
            return;
        }
        let path = std::env::temp_dir().join(format!(
            "aprendiendo-snapshot-migration-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        fs::copy(&source, &path).unwrap();
        let mut c = Connection::open(&path).unwrap();
        run(&mut c).unwrap();
        for (table, expected) in [
            ("sessions", 17),
            ("attempts", 24),
            ("observations", 162),
            ("weaknesses", 39),
            ("scheduler_items", 39),
            ("weakness_reviews", 0),
            ("review_observations", 0),
        ] {
            assert_eq!(
                c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
                expected
            );
        }
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM observations WHERE assessment_phase='historical' AND evidence_source='legacy' AND observation_no IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            162
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM recorded_requests WHERE replayable=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        let integrity: String = c
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        assert!(
            c.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none()
        );
        drop(c);
        let _ = fs::remove_file(path);
    }
}
