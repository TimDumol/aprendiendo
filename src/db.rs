use crate::model::{ObservationInput, RecordPracticeSessionRequest, UpsertWeaknessRequest};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

#[async_trait]
pub trait LearningStore: Send + Sync {
    async fn learning_context(&self, n: i32) -> Result<Value>;
    async fn recent_practice(&self, n: i32, s: Option<&str>) -> Result<Value>;
    async fn record_practice_json(&self, p: Value) -> Result<Value>;
    async fn review_queue(
        &self,
        n: i32,
        c: Option<&str>,
        as_of: Option<&str>,
        include_upcoming: bool,
    ) -> Result<Value>;
    async fn upsert_weakness_json(&self, p: Value) -> Result<Value>;
    async fn data_status(&self) -> Result<Value>;
    async fn ping(&self) -> Result<()>;
}
pub struct SqliteStore {
    connection: Mutex<Connection>,
}
impl SqliteStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        if let Some(d) = p.parent().filter(|x| !x.as_os_str().is_empty()) {
            fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
        }
        let c = Connection::open(p).with_context(|| format!("open {}", p.display()))?;
        c.execute_batch(include_str!("../sql/sqlite_schema.sql"))?;
        migrate_spaced_repetition(&c)?;
        Ok(Self {
            connection: Mutex::new(c),
        })
    }
    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("SQLite connection lock poisoned"))
    }
}
fn j(s: String) -> Result<Value> {
    Ok(serde_json::from_str(&s)?)
}

fn migrate_spaced_repetition(c: &Connection) -> Result<()> {
    let mut statement = c.prepare("PRAGMA table_info(weaknesses)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
    let migrations = [
        (
            "due_date",
            "ALTER TABLE weaknesses ADD COLUMN due_date TEXT",
        ),
        (
            "interval_days",
            "ALTER TABLE weaknesses ADD COLUMN interval_days INTEGER NOT NULL DEFAULT 0 CHECK (interval_days >= 0)",
        ),
        (
            "ease_factor",
            "ALTER TABLE weaknesses ADD COLUMN ease_factor REAL NOT NULL DEFAULT 2.5 CHECK (ease_factor >= 1.3)",
        ),
        (
            "repetitions",
            "ALTER TABLE weaknesses ADD COLUMN repetitions INTEGER NOT NULL DEFAULT 0 CHECK (repetitions >= 0)",
        ),
        (
            "lapses",
            "ALTER TABLE weaknesses ADD COLUMN lapses INTEGER NOT NULL DEFAULT 0 CHECK (lapses >= 0)",
        ),
        (
            "last_reviewed",
            "ALTER TABLE weaknesses ADD COLUMN last_reviewed TEXT",
        ),
    ];
    for (column, sql) in migrations {
        if !columns.contains(column) {
            c.execute(sql, [])?;
        }
    }
    c.execute(
        "UPDATE weaknesses SET due_date=COALESCE(due_date,last_seen,first_seen,date('now'))",
        [],
    )?;
    c.execute(
        "CREATE INDEX IF NOT EXISTS weaknesses_review_queue_idx ON weaknesses(active,due_date)",
        [],
    )?;
    Ok(())
}

fn update_review_schedules(
    c: &Connection,
    date: &str,
    session: &RecordPracticeSessionRequest,
) -> Result<()> {
    use crate::model::ObservationOutcome;
    use std::collections::HashMap;

    // A weakness advances at most once per session. If outcomes conflict, the least
    // successful result wins so one lucky attempt cannot hide a lapse.
    let mut grades: HashMap<&str, i32> = HashMap::new();
    let observations = session.observations.iter().chain(
        session
            .attempts
            .iter()
            .flat_map(|attempt| attempt.observations.iter()),
    );
    for observation in observations {
        let grade = match observation.outcome {
            ObservationOutcome::Correct => 2,
            ObservationOutcome::PromptedCorrect => 1,
            ObservationOutcome::Incorrect | ObservationOutcome::Omitted => 0,
        };
        grades
            .entry(&observation.weakness_key)
            .and_modify(|existing| *existing = (*existing).min(grade))
            .or_insert(grade);
    }
    for (key, grade) in grades {
        match grade {
            2 => {
                c.execute(
                    "UPDATE weaknesses SET repetitions=repetitions+1, interval_days=CASE repetitions WHEN 0 THEN 1 WHEN 1 THEN 3 ELSE max(1,round(interval_days*ease_factor)) END, ease_factor=min(3.0,ease_factor+0.1), last_reviewed=?2, due_date=date(?2,printf('+%d days',CASE repetitions WHEN 0 THEN 1 WHEN 1 THEN 3 ELSE max(1,round(interval_days*ease_factor)) END)) WHERE key=?1",
                    params![key, date],
                )?;
            }
            1 => {
                c.execute(
                    "UPDATE weaknesses SET repetitions=0, interval_days=1, ease_factor=max(1.3,ease_factor-0.15), last_reviewed=?2, due_date=date(?2,'+1 day') WHERE key=?1",
                    params![key, date],
                )?;
            }
            _ => {
                c.execute(
                    "UPDATE weaknesses SET repetitions=0, interval_days=1, lapses=lapses+1, ease_factor=max(1.3,ease_factor-0.2), last_reviewed=?2, due_date=date(?2,'+1 day') WHERE key=?1",
                    params![key, date],
                )?;
            }
        }
    }
    Ok(())
}

#[async_trait]
impl LearningStore for SqliteStore {
    async fn learning_context(&self, n: i32) -> Result<Value> {
        let c = self.conn()?;
        let w:String=c.query_row("SELECT COALESCE(json_group_array(json(x)),'[]') FROM (SELECT json_object('key',w.key,'category',w.category,'description',w.description,'target_pattern',w.target_pattern,'first_seen',w.first_seen,'last_seen',w.last_seen,'due_date',w.due_date,'interval_days',w.interval_days,'repetitions',w.repetitions,'lapses',w.lapses,'is_due',w.due_date<=date('now'),'incorrect_count',sum(o.outcome='incorrect'),'correct_count',sum(o.outcome='correct'),'observation_count',count(o.id)) x FROM weaknesses w LEFT JOIN observations o ON o.weakness_id=w.id WHERE w.active=1 GROUP BY w.id ORDER BY w.due_date<=date('now') DESC,w.due_date,sum(o.outcome='incorrect') DESC,w.key LIMIT 12)",[],|r|r.get(0))?;
        let s:String=c.query_row("SELECT COALESCE(json_group_array(json(x)),'[]') FROM (SELECT json_object('id',s.id,'session_date',s.session_date,'exercise_type',s.exercise_type,'topic',s.topic,'notes',s.notes,'attempt_count',(SELECT count(*) FROM attempts a WHERE a.session_id=s.id),'observation_count',(SELECT count(*) FROM observations o WHERE o.session_id=s.id)) x FROM sessions s ORDER BY s.session_date DESC,s.id DESC LIMIT ?1)",[n.clamp(1,20)],|r|r.get(0))?;
        Ok(json!({"active_weaknesses":j(w)?,"recent_sessions":j(s)?}))
    }
    async fn recent_practice(&self, n: i32, skill: Option<&str>) -> Result<Value> {
        let c = self.conn()?;
        let pattern = skill.map(|s| format!("%{s}%"));
        let mut q=c.prepare("SELECT id,session_date,exercise_type,topic,notes FROM sessions s WHERE ?1 IS NULL OR exercise_type LIKE ?1 COLLATE NOCASE OR COALESCE(topic,'') LIKE ?1 COLLATE NOCASE OR EXISTS(SELECT 1 FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.session_id=s.id AND (w.key=?2 OR w.category=?2)) ORDER BY session_date DESC,id DESC LIMIT ?3")?;
        let rows = q.query_map(params![pattern, skill, n.clamp(1, 50)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut items = vec![];
        for row in rows {
            let (id, date, kind, topic, notes) = row?;
            let a:String=c.query_row("SELECT COALESCE(json_group_array(json(x)),'[]') FROM (SELECT json_object('attempt_no',a.attempt_no,'transcript',substr(a.transcript,1,8000),'created_at',a.created_at,'observations',(SELECT COALESCE(json_group_array(json_object('weakness_key',w.key,'category',w.category,'outcome',o.outcome,'produced',o.produced,'correction',o.correction,'notes',o.notes)),'[]') FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.attempt_id=a.id ORDER BY o.id)) x FROM attempts a WHERE a.session_id=?1 ORDER BY a.attempt_no)",[id],|r|r.get(0))?;
            let o:String=c.query_row("SELECT COALESCE(json_group_array(json_object('weakness_key',w.key,'category',w.category,'outcome',o.outcome,'produced',o.produced,'correction',o.correction,'notes',o.notes)),'[]') FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.session_id=?1 AND o.attempt_id IS NULL ORDER BY o.id",[id],|r|r.get(0))?;
            items.push(json!({"id":id,"session_date":date,"exercise_type":kind,"topic":topic,"notes":notes,"attempts":j(a)?,"session_observations":j(o)?}));
        }
        Ok(json!({"items":items}))
    }
    async fn record_practice_json(&self, p: Value) -> Result<Value> {
        let x: RecordPracticeSessionRequest = serde_json::from_value(p)?;
        let mut c = self.conn()?;
        let tx = c.transaction()?;
        if let Some(id) = tx
            .query_row(
                "SELECT session_id FROM recorded_requests WHERE idempotency_key=?1",
                [&x.idempotency_key],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            return Ok(json!({"session_id":id,"recorded":false,"idempotent_replay":true}));
        }
        let mut all: Vec<&ObservationInput> = x.observations.iter().collect();
        for a in &x.attempts {
            all.extend(a.observations.iter());
        }
        for o in &all {
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM weaknesses WHERE key=?1)",
                [&o.weakness_key],
                |r| r.get::<_, bool>(0),
            )? {
                bail!("unknown weakness key: {}", o.weakness_key);
            }
        }
        let date = match x.session_date.clone() {
            Some(date) => date,
            None => tx.query_row("SELECT date('now')", [], |row| row.get(0))?,
        };
        let normalized = tx.query_row("SELECT date(?1)", [&date], |r| {
            r.get::<_, Option<String>>(0)
        })?;
        if normalized.as_deref() != Some(date.as_str()) {
            bail!("session_date must be a valid YYYY-MM-DD date");
        }
        tx.execute(
            "INSERT INTO sessions(session_date,exercise_type,topic,notes) VALUES(?1,?2,?3,?4)",
            params![date, x.exercise_type, x.topic, x.notes],
        )?;
        let sid = tx.last_insert_rowid();
        let mut count = 0;
        for a in &x.attempts {
            tx.execute(
                "INSERT INTO attempts(session_id,attempt_no,transcript) VALUES(?1,?2,?3)",
                params![sid, a.attempt_no, a.transcript],
            )?;
            let aid = tx.last_insert_rowid();
            for o in &a.observations {
                ins(&tx, sid, Some(aid), o)?;
                count += 1;
            }
        }
        for o in &x.observations {
            ins(&tx, sid, None, o)?;
            count += 1;
        }
        for o in all {
            tx.execute("UPDATE weaknesses SET first_seen=min(COALESCE(first_seen,?2),?2),last_seen=max(COALESCE(last_seen,?2),?2) WHERE key=?1",params![o.weakness_key,date])?;
        }
        update_review_schedules(&tx, &date, &x)?;
        tx.execute(
            "INSERT INTO recorded_requests(idempotency_key,session_id) VALUES(?1,?2)",
            params![x.idempotency_key, sid],
        )?;
        tx.commit()?;
        Ok(
            json!({"session_id":sid,"recorded":true,"idempotent_replay":false,"attempt_count":x.attempts.len(),"observation_count":count}),
        )
    }
    async fn review_queue(
        &self,
        n: i32,
        cat: Option<&str>,
        as_of: Option<&str>,
        include_upcoming: bool,
    ) -> Result<Value> {
        let c = self.conn()?;
        let review_date: String = match as_of {
            Some(value) => c
                .query_row("SELECT date(?1)", [value], |r| {
                    r.get::<_, Option<String>>(0)
                })?
                .ok_or_else(|| anyhow!("as_of must be a valid YYYY-MM-DD date"))?,
            None => c.query_row("SELECT date('now')", [], |r| r.get(0))?,
        };
        let s:String=c.query_row("SELECT COALESCE(json_group_array(json(x)),'[]') FROM (SELECT json_object('key',w.key,'category',w.category,'description',w.description,'target_pattern',w.target_pattern,'due_date',w.due_date,'is_due',w.due_date<=?2,'days_overdue',max(0,CAST(julianday(?2)-julianday(w.due_date) AS INTEGER)),'interval_days',w.interval_days,'ease_factor',round(w.ease_factor,2),'repetitions',w.repetitions,'lapses',w.lapses,'last_reviewed',w.last_reviewed,'incorrect_count',sum(o.outcome='incorrect'),'correct_count',sum(o.outcome='correct'),'observation_count',count(o.id),'error_rate',CASE WHEN count(o.id)=0 THEN 0 ELSE round(1.0*sum(o.outcome='incorrect')/count(o.id),3) END) x FROM weaknesses w LEFT JOIN observations o ON o.weakness_id=w.id WHERE w.active=1 AND (?1 IS NULL OR w.category=?1) AND (?3 OR w.due_date<=?2) GROUP BY w.id ORDER BY w.due_date<=?2 DESC,w.due_date,CASE WHEN count(o.id)=0 THEN 0 ELSE 1.0*sum(o.outcome='incorrect')/count(o.id) END DESC,w.key LIMIT ?4)",params![cat,review_date,include_upcoming,n.clamp(1,50)],|r|r.get(0))?;
        Ok(
            json!({"basis":"due weaknesses ordered by due date and historical error rate","as_of":review_date,"includes_upcoming":include_upcoming,"items":j(s)?}),
        )
    }
    async fn upsert_weakness_json(&self, p: Value) -> Result<Value> {
        let x: UpsertWeaknessRequest = serde_json::from_value(p)?;
        let c = self.conn()?;
        let old = c
            .query_row("SELECT id FROM weaknesses WHERE key=?1", [&x.key], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?;
        c.execute("INSERT INTO weaknesses(key,category,description,target_pattern,active,due_date) VALUES(?1,?2,?3,?4,COALESCE(?5,1),date('now')) ON CONFLICT(key) DO UPDATE SET category=excluded.category,description=excluded.description,target_pattern=excluded.target_pattern,active=COALESCE(?5,weaknesses.active)",params![x.key,x.category,x.description,x.target_pattern,x.active])?;
        let id = old.unwrap_or_else(|| c.last_insert_rowid());
        Ok(json!({"weakness_id":id,"key":x.key,"created":old.is_none(),"updated":old.is_some()}))
    }
    async fn data_status(&self) -> Result<Value> {
        let c = self.conn()?;
        let count = |t: &str| -> Result<i64> {
            Ok(c.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))?)
        };
        let last: Option<String> =
            c.query_row("SELECT max(session_date) FROM sessions", [], |r| r.get(0))?;
        Ok(
            json!({"schema_version":2,"storage_model":"single_learner","spaced_repetition":true,"last_session_date":last,"counts":{"sessions":count("sessions")?,"attempts":count("attempts")?,"observations":count("observations")?,"weaknesses":count("weaknesses")?,"active_weaknesses":c.query_row("SELECT count(*) FROM weaknesses WHERE active=1",[],|r|r.get::<_,i64>(0))?}}),
        )
    }
    async fn ping(&self) -> Result<()> {
        self.conn()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }
}
fn ins(c: &Connection, s: i64, a: Option<i64>, o: &ObservationInput) -> Result<()> {
    let out = serde_json::to_value(&o.outcome)?
        .as_str()
        .unwrap()
        .to_owned();
    c.execute("INSERT INTO observations(session_id,attempt_id,weakness_id,outcome,produced,correction,notes) SELECT ?1,?2,id,?3,?4,?5,?6 FROM weaknesses WHERE key=?7",params![s,a,out,o.produced,o.correction,o.notes,o.weakness_key])?;
    Ok(())
}

#[derive(Clone, Default)]
pub struct MockStore;
#[async_trait]
impl LearningStore for MockStore {
    async fn learning_context(&self, n: i32) -> Result<Value> {
        Ok(json!({"recent_sessions":n}))
    }
    async fn recent_practice(&self, n: i32, s: Option<&str>) -> Result<Value> {
        Ok(json!({"items":[],"limit":n,"skill":s}))
    }
    async fn record_practice_json(&self, p: Value) -> Result<Value> {
        Ok(json!({"recorded":true,"payload":p}))
    }
    async fn review_queue(
        &self,
        n: i32,
        c: Option<&str>,
        as_of: Option<&str>,
        include_upcoming: bool,
    ) -> Result<Value> {
        Ok(
            json!({"items":[],"limit":n,"category":c,"as_of":as_of,"include_upcoming":include_upcoming}),
        )
    }
    async fn upsert_weakness_json(&self, p: Value) -> Result<Value> {
        Ok(json!({"updated":true,"patch":p}))
    }
    async fn data_status(&self) -> Result<Value> {
        Ok(json!({"schema_version":1,"status":"ready"}))
    }
    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}
pub type SharedStore = Arc<dyn LearningStore>;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteStore {
        let path =
            std::env::temp_dir().join(format!("aprendiendo-{}.sqlite3", uuid::Uuid::new_v4()));
        SqliteStore::new(path).expect("test store should open")
    }

    async fn add_weakness(store: &SqliteStore) {
        store
            .upsert_weakness_json(json!({
                "key": "ser-estar",
                "category": "grammar",
                "description": "Choose ser or estar",
                "target_pattern": "ser vs estar"
            }))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn observations_drive_a_spaced_review_schedule() {
        let store = test_store();
        add_weakness(&store).await;

        store
            .record_practice_json(json!({
                "idempotency_key": "correct-1",
                "session_date": "2026-08-28",
                "exercise_type": "recall",
                "attempts": [{
                    "attempt_no": 1,
                    "transcript": "Está cansado",
                    "observations": [
                        {"weakness_key": "ser-estar", "outcome": "correct"},
                        {"weakness_key": "ser-estar", "outcome": "correct"}
                    ]
                }]
            }))
            .await
            .unwrap();

        let first = store
            .review_queue(10, None, Some("2026-08-28"), true)
            .await
            .unwrap();
        assert_eq!(first["items"][0]["repetitions"], 1);
        assert_eq!(first["items"][0]["interval_days"], 1);
        assert_eq!(first["items"][0]["due_date"], "2026-08-29");

        store
            .record_practice_json(json!({
                "idempotency_key": "lapse-1",
                "session_date": "2026-08-29",
                "exercise_type": "recall",
                "observations": [{"weakness_key": "ser-estar", "outcome": "incorrect"}]
            }))
            .await
            .unwrap();

        let after_lapse = store
            .review_queue(10, None, Some("2026-08-29"), true)
            .await
            .unwrap();
        assert_eq!(after_lapse["items"][0]["repetitions"], 0);
        assert_eq!(after_lapse["items"][0]["lapses"], 1);
        assert_eq!(after_lapse["items"][0]["due_date"], "2026-08-30");
    }

    #[test]
    fn upgrades_an_existing_weakness_catalog_without_losing_it() {
        let path =
            std::env::temp_dir().join(format!("aprendiendo-{}.sqlite3", uuid::Uuid::new_v4()));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE weaknesses (
                id INTEGER PRIMARY KEY,
                key TEXT NOT NULL UNIQUE,
                category TEXT NOT NULL,
                description TEXT NOT NULL,
                target_pattern TEXT,
                first_seen TEXT,
                last_seen TEXT,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
             );
             INSERT INTO weaknesses(key,category,description,last_seen,created_at)
             VALUES('por-para','grammar','Choose por or para','2026-08-20','2026-08-20');",
            )
            .unwrap();
        drop(connection);

        let store = SqliteStore::new(path).expect("legacy database should upgrade");
        let connection = store.conn().unwrap();
        let row: (String, i64) = connection
            .query_row(
                "SELECT due_date,repetitions FROM weaknesses WHERE key='por-para'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("2026-08-20".into(), 0));
    }
}
