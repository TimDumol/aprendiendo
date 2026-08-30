use crate::{
    evidence, fsrs_adapter, migrations,
    model::{
        DetailMode, DrillMix, EvidenceSource, EvidenceStrength, FsrsRating, HintLevel,
        LearningContextRequest, NewWeaknessInput, ObservationInput, PracticeBriefRequest,
        PracticeObjective, RatingSource, RecentPracticeRequest, RecordPracticeSessionRequest,
        RetrievalMode, ReviewQueueRequest, TargetRelationInput, TargetType, TaxonomyRequest,
        UpsertConceptRequest, UpsertWeaknessRequest,
    },
    taxonomy,
};
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OptionalExtension, params, types::Value as SqlValue};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

#[async_trait]
pub trait LearningStore: Send + Sync {
    async fn learning_context(&self, request: LearningContextRequest) -> Result<Value>;
    async fn recent_practice(&self, request: RecentPracticeRequest) -> Result<Value>;
    async fn record_practice_json(&self, payload: Value) -> Result<Value>;
    async fn review_queue(&self, request: ReviewQueueRequest) -> Result<Value>;
    async fn practice_brief(&self, request: PracticeBriefRequest) -> Result<Value>;
    async fn upsert_weakness_json(&self, payload: Value) -> Result<Value>;
    async fn taxonomy(&self, request: TaxonomyRequest) -> Result<Value>;
    async fn upsert_concept_json(&self, payload: Value) -> Result<Value>;
    async fn data_status(&self) -> Result<Value>;
    async fn ping(&self) -> Result<()>;
}

pub struct SqliteStore {
    connection: Mutex<Connection>,
}

impl SqliteStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let p = path.as_ref();
        if let Some(parent) = p.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let mut c = Connection::open(p).with_context(|| format!("open {}", p.display()))?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=10000;")?;
        migrations::run(&mut c)?;
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

#[derive(Debug, Clone)]
struct SchedulerConfig {
    desired_retention: f32,
    parameters: Vec<f32>,
    timezone: String,
    cutoff: u32,
    parameter_set_id: i64,
    algorithm: String,
    algorithm_version: String,
    source: String,
}
fn scheduler_config(c: &Connection) -> Result<SchedulerConfig> {
    c.query_row("SELECT p.algorithm,p.algorithm_version,p.parameters_json,p.source,c.desired_retention,c.timezone,c.day_cutoff_hour,c.active_parameter_set_id FROM scheduler_config c JOIN scheduler_parameter_sets p ON p.id=c.active_parameter_set_id WHERE c.id=1",[],|r| {
        let raw:String=r.get(2)?; let parameters=serde_json::from_str(&raw).map_err(|e|rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        Ok(SchedulerConfig{algorithm:r.get(0)?,algorithm_version:r.get(1)?,parameters,source:r.get(3)?,desired_retention:r.get::<_,f64>(4)? as f32,timezone:r.get(5)?,cutoff:r.get::<_,i64>(6)? as u32,parameter_set_id:r.get(7)?})
    }).map_err(Into::into)
}
fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
fn normalized_timestamp(value: Option<&str>) -> Result<(String, DateTime<Utc>)> {
    let dt = value
        .map(|v| {
            DateTime::parse_from_rfc3339(v)
                .map(|x| x.with_timezone(&Utc))
                .map_err(|_| anyhow!("reviewed_at must be a valid ISO-8601 timestamp"))
        })
        .transpose()?
        .unwrap_or_else(now_utc);
    Ok((dt.to_rfc3339_opts(SecondsFormat::Millis, true), dt))
}
fn parse_timezone(name: &str) -> Result<Tz> {
    name.parse::<Tz>()
        .map_err(|_| anyhow!("invalid IANA scheduler timezone: {name}"))
}
pub(crate) fn learning_day(dt: DateTime<Utc>, timezone: &str, cutoff: u32) -> Result<NaiveDate> {
    let local = dt.with_timezone(&parse_timezone(timezone)?);
    Ok(if local.hour() < cutoff {
        local.date_naive() - Duration::days(1)
    } else {
        local.date_naive()
    })
}
fn learning_day_delta(previous: &str, current: &str, timezone: &str, cutoff: u32) -> Result<i64> {
    let a = DateTime::parse_from_rfc3339(previous)?.with_timezone(&Utc);
    let b = DateTime::parse_from_rfc3339(current)?.with_timezone(&Utc);
    let delta =
        (learning_day(b, timezone, cutoff)? - learning_day(a, timezone, cutoff)?).num_days();
    if delta < 0 {
        bail!("reviewed_at must not be earlier than the scheduler item's last review");
    }
    Ok(delta)
}
fn valid_date(value: &str, field: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| anyhow!("{field} must be a valid YYYY-MM-DD date"))
}
fn serialize_enum<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("enum serialization failed"))
}
fn hash_payload(payload: &Value) -> Result<String> {
    fn canonical(v: &Value) -> Value {
        match v {
            Value::Object(o) => {
                let mut out = Map::new();
                let mut keys = o.keys().collect::<Vec<_>>();
                keys.sort();
                for k in keys {
                    out.insert(k.clone(), canonical(&o[k]));
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
            x => x.clone(),
        }
    }
    let bytes = serde_json::to_vec(&canonical(payload))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn prompt_fingerprint(prompt: &str) -> String {
    let normalized = prompt
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone)]
struct WeaknessRow {
    id: i64,
    scheduler_id: Option<i64>,
    key: String,
    category: String,
    description: String,
    target_pattern: Option<String>,
    pending: bool,
    due: Option<String>,
    stability: Option<f64>,
    difficulty: Option<f64>,
    last_review: Option<String>,
    last_rating: Option<i32>,
    review_count: i64,
    first_seen: Option<String>,
    last_seen: Option<String>,
    legacy_due: Option<String>,
    legacy_interval: i64,
    legacy_ease: f64,
    legacy_repetitions: i64,
    legacy_lapses: i64,
    incorrect_count: i64,
    correct_count: i64,
    observation_count: i64,
    error_days: i64,
    latest_error: Option<String>,
    unresolved: i64,
    branch: String,
}

fn target_type_str(t: TargetType) -> &'static str {
    t.as_str()
}
fn target_type_list(types: Option<&[TargetType]>) -> Vec<String> {
    types
        .unwrap_or_default()
        .iter()
        .map(|x| target_type_str(*x).to_owned())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn load_weaknesses(
    c: &Connection,
    categories: Option<&[String]>,
    keys: Option<&[String]>,
    concepts: Option<&[String]>,
    schemes: Option<&[String]>,
    collections: Option<&[String]>,
    target_types: Option<&[TargetType]>,
    include_nonactive: bool,
) -> Result<Vec<WeaknessRow>> {
    let mut sql="SELECT w.id,si.id,w.key,w.category,w.description,w.target_pattern,w.target_type,w.target_status,w.classification_pending,(SELECT c.key FROM weakness_concepts wc JOIN concepts c ON c.id=wc.concept_id WHERE wc.weakness_id=w.id AND wc.role='primary'),si.due_learning_day,si.stability,si.difficulty,si.last_review_at,(SELECT r.rating FROM weakness_reviews r WHERE r.scheduler_item_id=si.id ORDER BY r.reviewed_at DESC,r.id DESC LIMIT 1),(SELECT count(*) FROM weakness_reviews r WHERE r.scheduler_item_id=si.id),w.first_seen,w.last_seen,w.due_date,w.interval_days,w.ease_factor,w.repetitions,w.lapses,(SELECT count(*) FROM observations o WHERE o.weakness_id=w.id AND o.outcome IN ('incorrect','omitted')),(SELECT count(*) FROM observations o WHERE o.weakness_id=w.id AND o.outcome IN ('correct','prompted_correct')),(SELECT count(*) FROM observations o WHERE o.weakness_id=w.id),(SELECT count(DISTINCT s.session_date) FROM observations o JOIN sessions s ON s.id=o.session_id WHERE o.weakness_id=w.id AND o.outcome IN ('incorrect','omitted')),(SELECT max(s.session_date) FROM observations o JOIN sessions s ON s.id=o.session_id WHERE o.weakness_id=w.id AND o.outcome IN ('incorrect','omitted')),(SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM observations bad JOIN sessions sb ON sb.id=bad.session_id WHERE bad.weakness_id=w.id AND bad.outcome IN ('incorrect','omitted') AND sb.session_date > (SELECT max(sx.session_date) FROM observations bx JOIN sessions sx ON sx.id=bx.session_id WHERE bx.weakness_id=w.id AND bx.outcome IN ('incorrect','omitted'))) THEN 1 ELSE 0 END),COALESCE((SELECT c.key FROM weakness_concepts wc JOIN concepts c ON c.id=wc.concept_id WHERE wc.weakness_id=w.id AND wc.role='primary'),'') FROM weaknesses w LEFT JOIN scheduler_items si ON si.weakness_id=w.id AND si.track='general' ".to_owned();
    let mut vals: Vec<SqlValue> = Vec::new();
    sql.push_str(if include_nonactive {
        "WHERE 1=1"
    } else {
        "WHERE w.target_status='active' AND (si.active=1 OR si.id IS NULL)"
    });
    let add_in = |sql: &mut String, vals: &mut Vec<SqlValue>, prefix: &str, items: &[String]| {
        if !items.is_empty() {
            sql.push_str(prefix);
            sql.push('(');
            sql.push_str(
                &std::iter::repeat_n("?", items.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push(')');
            vals.extend(items.iter().cloned().map(SqlValue::from));
        }
    };
    if let Some(v) = categories.filter(|v| !v.is_empty()) {
        add_in(&mut sql, &mut vals, " AND w.category IN ", v);
    }
    if let Some(v) = keys.filter(|v| !v.is_empty()) {
        add_in(&mut sql, &mut vals, " AND w.key IN ", v);
    }
    let types = target_type_list(target_types);
    if !types.is_empty() {
        add_in(&mut sql, &mut vals, " AND w.target_type IN ", &types);
    }
    if let Some(v) = concepts.filter(|v| !v.is_empty()) {
        let ids = taxonomy::descendant_ids(c, v, 6)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        sql.push_str(" AND EXISTS(SELECT 1 FROM weakness_concepts f WHERE f.weakness_id=w.id AND f.concept_id IN (");
        sql.push_str(
            &std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(","),
        );
        sql.push(')');
        vals.extend(ids.into_iter().map(SqlValue::from));
    }
    if let Some(v) = schemes.filter(|v| !v.is_empty()) {
        add_in(
            &mut sql,
            &mut vals,
            " AND EXISTS(SELECT 1 FROM weakness_concepts f JOIN concepts cc ON cc.id=f.concept_id JOIN concept_schemes cs ON cs.id=cc.scheme_id WHERE f.weakness_id=w.id AND cs.key IN ",
            v,
        );
        sql.push(')');
    }
    if let Some(v) = collections.filter(|v| !v.is_empty()) {
        add_in(
            &mut sql,
            &mut vals,
            " AND EXISTS(SELECT 1 FROM collection_members cm JOIN collections co ON co.id=cm.collection_id WHERE cm.weakness_id=w.id AND co.key IN ",
            v,
        );
        sql.push(')');
    }
    sql.push_str(" ORDER BY w.id");
    let mut stmt = c.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(vals.iter()), |r| {
            Ok(WeaknessRow {
                id: r.get(0)?,
                scheduler_id: r.get(1)?,
                key: r.get(2)?,
                category: r.get(3)?,
                description: r.get(4)?,
                target_pattern: r.get(5)?,
                pending: r.get::<_, i64>(8)? != 0,
                due: r.get(10)?,
                stability: r.get(11)?,
                difficulty: r.get(12)?,
                last_review: r.get(13)?,
                last_rating: r.get(14)?,
                review_count: r.get(15)?,
                first_seen: r.get(16)?,
                last_seen: r.get(17)?,
                legacy_due: r.get(18)?,
                legacy_interval: r.get(19)?,
                legacy_ease: r.get(20)?,
                legacy_repetitions: r.get(21)?,
                legacy_lapses: r.get(22)?,
                incorrect_count: r.get(23)?,
                correct_count: r.get(24)?,
                observation_count: r.get(25)?,
                error_days: r.get(26)?,
                latest_error: r.get(27)?,
                unresolved: r.get(28)?,
                branch: r.get(29)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn is_due(row: &WeaknessRow, day: NaiveDate) -> bool {
    row.due
        .as_deref()
        .map(|d| {
            valid_date(d, "due_learning_day")
                .map(|x| x <= day)
                .unwrap_or(true)
        })
        .unwrap_or(true)
}
fn row_retrievability(
    row: &WeaknessRow,
    day: NaiveDate,
    cfg: &SchedulerConfig,
) -> Result<Option<f64>> {
    let (Some(s), Some(d), Some(last)) =
        (row.stability, row.difficulty, row.last_review.as_deref())
    else {
        return Ok(None);
    };
    let state = fsrs_adapter::memory_state(s, d)?;
    let last = DateTime::parse_from_rfc3339(last)?.with_timezone(&Utc);
    let elapsed = (day - learning_day(last, &cfg.timezone, cfg.cutoff)?)
        .num_days()
        .max(0);
    Ok(Some(f64::from(fsrs_adapter::retrievability(
        state,
        elapsed,
        &cfg.parameters,
    )?)))
}
fn sort_rows(rows: &mut [WeaknessRow], day: NaiveDate, cfg: &SchedulerConfig) -> Result<()> {
    let mut ret = HashMap::new();
    for r in rows.iter() {
        ret.insert(r.id, row_retrievability(r, day, cfg)?);
    }
    rows.sort_by(|a, b| {
        let ai = a.due.is_some();
        let bi = b.due.is_some();
        bi.cmp(&ai)
            .then_with(|| {
                if ai && bi {
                    a.due.cmp(&b.due)
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| {
                if !ai && !bi {
                    b.error_days
                        .cmp(&a.error_days)
                        .then(b.unresolved.cmp(&a.unresolved))
                        .then(b.latest_error.cmp(&a.latest_error))
                        .then(b.incorrect_count.cmp(&a.incorrect_count))
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| match (ret[&a.id], ret[&b.id]) {
                (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(())
}
fn diversify_rows(rows: &mut [WeaknessRow]) {
    let mut start = 0;
    while start < rows.len() {
        let due = rows[start].due.clone();
        let end = rows[start..]
            .iter()
            .position(|row| row.due != due)
            .map_or(rows.len(), |offset| start + offset);
        diversify_segment(&mut rows[start..end]);
        start = end;
    }
}

fn diversify_segment(rows: &mut [WeaknessRow]) {
    let mut out = Vec::with_capacity(rows.len());
    let mut used = HashSet::new();
    for row in rows.iter().cloned() {
        if used.contains(&row.branch) {
            out.push(row);
        } else {
            let pos = out
                .iter()
                .position(|x: &WeaknessRow| !used.contains(&x.branch))
                .unwrap_or(out.len());
            out.insert(pos, row.clone());
            used.insert(row.branch.clone());
        }
    }
    rows.clone_from_slice(&out);
}

fn classification(c: &Connection, wid: i64, full: bool) -> Result<Value> {
    let primary:Option<(String,String)>=c.query_row("SELECT c.key,c.label FROM weakness_concepts wc JOIN concepts c ON c.id=wc.concept_id WHERE wc.weakness_id=?1 AND wc.role='primary'",[wid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let pending: bool = c.query_row(
        "SELECT classification_pending FROM weaknesses WHERE id=?1",
        [wid],
        |r| Ok(r.get::<_, i64>(0)? != 0),
    )?;
    let mut stmt=c.prepare("SELECT c.key,c.label,wc.role FROM weakness_concepts wc JOIN concepts c ON c.id=wc.concept_id WHERE wc.weakness_id=?1 AND wc.role<>'primary' ORDER BY wc.role,c.key")?;
    let concepts = stmt
        .query_map([wid], |r| {
            let key: String = r.get(0)?;
            let label: String = r.get(1)?;
            let role: String = r.get(2)?;
            Ok(if full {
                json!({"key":key,"label":label,"role":role})
            } else {
                json!({"key":key,"role":role})
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt=c.prepare("SELECT co.key,co.label FROM collection_members cm JOIN collections co ON co.id=cm.collection_id WHERE cm.weakness_id=?1 AND co.active=1 ORDER BY cm.position IS NULL,cm.position,co.key")?;
    let cols = stmt
        .query_map([wid], |r| {
            Ok(json!({"key":r.get::<_,String>(0)?,"label":r.get::<_,String>(1)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"target_type":c.query_row("SELECT target_type FROM weaknesses WHERE id=?1",[wid],|r|r.get::<_,String>(0))?,"target_status":c.query_row("SELECT target_status FROM weaknesses WHERE id=?1",[wid],|r|r.get::<_,String>(0))?,"classification_pending":pending,"primary_concept":primary.map(|(key,label)|json!({"key":key,"label":label})),"concepts":concepts,"collections":cols}),
    )
}
fn fsrs_json(row: &WeaknessRow, day: NaiveDate, cfg: &SchedulerConfig) -> Result<Value> {
    Ok(
        json!({"due_learning_day":row.due,"retrievability":row_retrievability(row,day,cfg)?,"stability":row.stability,"difficulty":row.difficulty,"last_reviewed_at":row.last_review,"last_rating":row.last_rating.map(rating_name),"review_count":row.review_count}),
    )
}
fn rating_name(n: i32) -> &'static str {
    match n {
        1 => "again",
        2 => "hard",
        3 => "good",
        4 => "easy",
        _ => "unknown",
    }
}

fn observation_json(
    c: &Connection,
    sid: i64,
    aid: Option<i64>,
    iid: Option<i64>,
) -> Result<Vec<Value>> {
    let mut sql="SELECT w.key,w.category,o.observation_no,o.outcome,o.role,o.assessment_phase,o.hint_level,o.learner_effort,o.evidence_source,o.evidence_strength,o.severity,o.produced,o.correction,o.error_span,o.notes FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.session_id=?1".to_owned();
    let bound = aid.or(iid);
    if aid.is_some() {
        sql.push_str(" AND o.attempt_id=?2")
    } else if iid.is_some() {
        sql.push_str(" AND o.practice_item_id=?2 AND o.attempt_id IS NULL")
    } else {
        sql.push_str(" AND o.attempt_id IS NULL AND o.practice_item_id IS NULL")
    }
    sql.push_str(" ORDER BY o.observation_no");
    let mut stmt = c.prepare(&sql)?;
    let mut rows = match bound {
        Some(id) => stmt.query(params![sid, id])?,
        None => stmt.query(params![sid])?,
    };
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(json!({"weakness_key":r.get::<_,String>(0)?,"category":r.get::<_,String>(1)?,"observation_no":r.get::<_,i64>(2)?,"outcome":r.get::<_,String>(3)?,"role":r.get::<_,String>(4)?,"assessment_phase":r.get::<_,String>(5)?,"hint_level":r.get::<_,String>(6)?,"learner_effort":r.get::<_,Option<String>>(7)?,"evidence_source":r.get::<_,String>(8)?,"evidence_strength":r.get::<_,Option<String>>(9)?,"severity":r.get::<_,Option<String>>(10)?,"produced":r.get::<_,Option<String>>(11)?,"correction":r.get::<_,Option<String>>(12)?,"error_span":r.get::<_,Option<String>>(13)?,"notes":r.get::<_,Option<String>>(14)?}));
    }
    Ok(out)
}
fn fetch_recent_prompts(c: &Connection, wid: i64, limit: i32) -> Result<Vec<String>> {
    let mut s=c.prepare("SELECT prompt FROM practice_items pi JOIN practice_item_targets t ON t.practice_item_id=pi.id WHERE t.weakness_id=?1 GROUP BY prompt ORDER BY max(created_at) DESC,prompt LIMIT ?2")?;
    Ok(s.query_map(params![wid, limit], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
fn recent_errors(c: &Connection, wid: i64) -> Result<Vec<Value>> {
    let mut s=c.prepare("SELECT produced,correction FROM observations WHERE weakness_id=?1 AND outcome IN ('incorrect','partially_correct','omitted') AND (produced IS NOT NULL OR correction IS NOT NULL) ORDER BY id DESC LIMIT 3")?;
    Ok(s.query_map([wid],|r|Ok(json!({"produced":r.get::<_,Option<String>>(0)?,"correction":r.get::<_,Option<String>>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn recommendations(c: &Connection, wid: i64) -> Result<Vec<Value>> {
    let mut stmt=c.prepare("SELECT drill_type,stage,weight FROM weakness_drill_overrides WHERE weakness_id=?1 ORDER BY weight DESC,drill_type")?;
    let direct=stmt.query_map([wid],|r|Ok(json!({"drill_type":r.get::<_,String>(0)?,"stage":r.get::<_,String>(1)?,"weight":r.get::<_,f64>(2)?,"source":"target_override"})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if !direct.is_empty() {
        return Ok(direct);
    }
    let mut s=c.prepare("WITH RECURSIVE linked(id,depth) AS (SELECT concept_id,0 FROM weakness_concepts WHERE weakness_id=?1 UNION SELECT e.object_id,linked.depth+1 FROM concept_edges e JOIN linked ON linked.id=e.subject_id WHERE e.predicate='broader' AND linked.depth<6) SELECT d.drill_type,d.stage,d.weight,co.key FROM linked JOIN concept_drill_recommendations d ON d.concept_id=linked.id JOIN concepts co ON co.id=linked.id ORDER BY linked.depth,d.weight DESC,d.drill_type")?;
    let mut best: HashMap<String, Value> = HashMap::new();
    for row in s.query_map([wid],|r|Ok(json!({"drill_type":r.get::<_,String>(0)?,"stage":r.get::<_,String>(1)?,"weight":r.get::<_,f64>(2)?,"source_concept":r.get::<_,String>(3)?})))?{let v=row?;let key=v["drill_type"].as_str().unwrap().to_owned();best.entry(key).or_insert(v);}
    Ok(best.into_values().collect())
}

#[async_trait]
impl LearningStore for SqliteStore {
    async fn learning_context(&self, request: LearningContextRequest) -> Result<Value> {
        let c = self.conn()?;
        let cfg = scheduler_config(&c)?;
        let day = learning_day(now_utc(), &cfg.timezone, cfg.cutoff)?;
        let mut rows = load_weaknesses(
            &c,
            None,
            None,
            request.concept_keys.as_deref(),
            request.scheme_keys.as_deref(),
            request.collection_keys.as_deref(),
            request.target_types.as_deref(),
            false,
        )?;
        sort_rows(&mut rows, day, &cfg)?;
        rows.truncate(usize::from(
            request.weakness_limit.unwrap_or(12).clamp(1, 50),
        ));
        let full = matches!(request.detail, Some(DetailMode::Full));
        let weaknesses=rows.into_iter().map(|r|Ok(json!({"key":r.key,"category":r.category,"description":r.description,"target_pattern":r.target_pattern,"first_seen":r.first_seen,"last_seen":r.last_seen,"classification":classification(&c,r.id,full)?,"fsrs":fsrs_json(&r,day,&cfg)?,"incorrect_count":r.incorrect_count,"correct_count":r.correct_count,"observation_count":r.observation_count,"recent_prompts":fetch_recent_prompts(&c,r.id,if full{4}else{2})?}))).collect::<Result<Vec<_>>>()?;
        let n = i32::from(request.recent_sessions.unwrap_or(5).clamp(1, 20));
        let mut s=c.prepare("SELECT id,session_date,exercise_type,exercise_type_key,topic,notes,(SELECT count(*) FROM attempts a WHERE a.session_id=s.id),(SELECT count(*) FROM observations o WHERE o.session_id=s.id),(SELECT count(*) FROM practice_items p WHERE p.session_id=s.id) FROM sessions s ORDER BY session_date DESC,id DESC LIMIT ?1")?;
        let sessions=s.query_map([n],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"session_date":r.get::<_,String>(1)?,"exercise_type":r.get::<_,String>(2)?,"exercise_type_key":r.get::<_,Option<String>>(3)?,"topic":r.get::<_,Option<String>>(4)?,"notes":r.get::<_,Option<String>>(5)?,"attempt_count":r.get::<_,i64>(6)?,"observation_count":r.get::<_,i64>(7)?,"item_count":r.get::<_,i64>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(
            json!({"as_of":day.to_string(),"active_weaknesses":weaknesses,"recent_sessions":sessions,"scheduler":{"algorithm":cfg.algorithm,"algorithm_version":cfg.algorithm_version,"desired_retention":cfg.desired_retention,"parameter_set_id":cfg.parameter_set_id,"parameters_source":cfg.source}}),
        )
    }

    async fn recent_practice(&self, request: RecentPracticeRequest) -> Result<Value> {
        let c = self.conn()?;
        let full = matches!(request.detail, Some(DetailMode::Full));
        let include_items = request.include_items.unwrap_or(full);
        let include_attempts = request.include_attempts.unwrap_or(full);
        let include_observations = request.include_observations.unwrap_or(full);
        let from = request
            .from_date
            .as_deref()
            .map(|x| valid_date(x, "from_date"))
            .transpose()?;
        let to = request
            .to_date
            .as_deref()
            .map(|x| valid_date(x, "to_date"))
            .transpose()?;
        if let (Some(a), Some(b)) = (from, to)
            && a > b
        {
            bail!("from_date must not be after to_date")
        }
        let mut sql="SELECT DISTINCT s.id,s.session_date,s.exercise_type,s.exercise_type_key,s.topic,s.notes FROM sessions s WHERE 1=1".to_owned();
        let mut vals: Vec<SqlValue> = Vec::new();
        let target_union = "(SELECT o.weakness_id FROM observations o WHERE o.session_id=s.id UNION SELECT t.weakness_id FROM practice_item_targets t JOIN practice_items p ON p.id=t.practice_item_id WHERE p.session_id=s.id)";
        if let Some(v) = from {
            sql.push_str(" AND s.session_date>=?");
            vals.push(v.to_string().into())
        }
        if let Some(v) = to {
            sql.push_str(" AND s.session_date<=?");
            vals.push(v.to_string().into())
        }
        if let Some(v) = request.exercise_types.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND s.exercise_type IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push(')');
            vals.extend(v.iter().cloned().map(SqlValue::from));
        }
        if let Some(v) = request.drill_types.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND EXISTS(SELECT 1 FROM practice_items p WHERE p.session_id=s.id AND p.drill_type IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(v.iter().map(|x| SqlValue::from(x.as_str().to_owned())));
        }
        if let Some(v) = request.weakness_keys.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND EXISTS(SELECT 1 FROM ");
            sql.push_str(target_union);
            sql.push_str(" sw JOIN weaknesses w ON w.id=sw.weakness_id WHERE w.key IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(v.iter().cloned().map(SqlValue::from));
        }
        if let Some(v) = request.categories.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND EXISTS(SELECT 1 FROM ");
            sql.push_str(target_union);
            sql.push_str(" sw JOIN weaknesses w ON w.id=sw.weakness_id WHERE w.category IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(v.iter().cloned().map(SqlValue::from));
        }
        if let Some(v) = request.concept_keys.as_ref().filter(|v| !v.is_empty()) {
            let ids = taxonomy::descendant_ids(&c, v, 6)?;
            if ids.is_empty() {
                sql.push_str(" AND 0=1");
            } else {
                sql.push_str(" AND EXISTS(SELECT 1 FROM ");
                sql.push_str(target_union);
                sql.push_str(" sw JOIN weakness_concepts wc ON wc.weakness_id=sw.weakness_id WHERE wc.concept_id IN (");
                sql.push_str(
                    &std::iter::repeat_n("?", ids.len())
                        .collect::<Vec<_>>()
                        .join(","),
                );
                sql.push_str("))");
                vals.extend(ids.into_iter().map(SqlValue::from));
            }
        }
        if let Some(v) = request.scheme_keys.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND EXISTS(SELECT 1 FROM ");
            sql.push_str(target_union);
            sql.push_str(" sw JOIN weakness_concepts wc ON wc.weakness_id=sw.weakness_id JOIN concepts c2 ON c2.id=wc.concept_id JOIN concept_schemes cs2 ON cs2.id=c2.scheme_id WHERE cs2.key IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(v.iter().cloned().map(SqlValue::from));
        }
        if let Some(v) = request.collection_keys.as_ref().filter(|v| !v.is_empty()) {
            sql.push_str(" AND EXISTS(SELECT 1 FROM ");
            sql.push_str(target_union);
            sql.push_str(" sw JOIN collection_members cm ON cm.weakness_id=sw.weakness_id JOIN collections co ON co.id=cm.collection_id WHERE co.key IN (");
            sql.push_str(
                &std::iter::repeat_n("?", v.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(v.iter().cloned().map(SqlValue::from));
        }
        if let Some(v) = request.target_types.as_ref().filter(|v| !v.is_empty()) {
            let types = target_type_list(Some(v));
            sql.push_str(" AND EXISTS(SELECT 1 FROM ");
            sql.push_str(target_union);
            sql.push_str(
                " sw JOIN weaknesses w2 ON w2.id=sw.weakness_id WHERE w2.target_type IN (",
            );
            sql.push_str(
                &std::iter::repeat_n("?", types.len())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            sql.push_str("))");
            vals.extend(types.into_iter().map(SqlValue::from));
        }
        if let Some(skill) = request.skill.as_deref() {
            sql.push_str(" AND (s.exercise_type LIKE ? OR COALESCE(s.topic,'') LIKE ? OR EXISTS(SELECT 1 FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.session_id=s.id AND (w.key=? OR w.category=?)))");
            let p = format!("%{skill}%");
            vals.extend(
                [p.clone(), p, skill.to_owned(), skill.to_owned()]
                    .into_iter()
                    .map(SqlValue::from),
            );
        }
        sql.push_str(" ORDER BY s.session_date DESC,s.id DESC LIMIT ?");
        vals.push(i64::from(request.limit.unwrap_or(10)).into());
        let mut st = c.prepare(&sql)?;
        let sessions = st
            .query_map(rusqlite::params_from_iter(vals.iter()), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut result = Vec::new();
        for (sid, date, kind, type_key, topic, notes) in sessions {
            let mut item = json!({"id":sid,"session_date":date,"exercise_type":kind,"exercise_type_key":type_key,"topic":topic,"notes":notes});
            if include_items {
                let mut q=c.prepare("SELECT id,item_no,drill_type,prompt,response,corrected_response,reference_answer,feedback,outcome FROM practice_items WHERE session_id=?1 ORDER BY item_no")?;
                let mut arr = Vec::new();
                for row in q.query_map([sid], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<String>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                    ))
                })? {
                    let (iid, no, drill, prompt, response, corrected, reference, feedback, outcome) =
                        row?;
                    let targets:Vec<String>=c.prepare("SELECT w.key FROM practice_item_targets t JOIN weaknesses w ON w.id=t.weakness_id WHERE t.practice_item_id=?1 ORDER BY w.key")?.query_map([iid],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    let mut v = json!({"item_no":no,"drill_type":drill,"prompt":prompt,"prompt_fingerprint":c.query_row("SELECT prompt_fingerprint FROM practice_items WHERE id=?1",[iid],|r|r.get::<_,Option<String>>(0))?,"response":response,"corrected_response":corrected,"reference_answer":reference,"feedback":feedback,"outcome":outcome,"target_weakness_keys":targets});
                    if include_observations {
                        v["observations"] = json!(observation_json(&c, sid, None, Some(iid))?);
                    }
                    arr.push(v)
                }
                item["items"] = json!(arr);
            }
            if include_attempts {
                let mut q=c.prepare("SELECT id,attempt_no,practice_item_id,transcript,target_duration_seconds,actual_duration_milliseconds,created_at FROM attempts WHERE session_id=?1 ORDER BY attempt_no")?;
                let mut arr = Vec::new();
                for row in q.query_map([sid], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                })? {
                    let (aid, no, iid, transcript, target, actual, created) = row?;
                    let mut v = json!({"attempt_no":no,"practice_item_no":iid.and_then(|id|c.query_row("SELECT item_no FROM practice_items WHERE id=?1",[id],|r|r.get::<_,i64>(0)).ok()),"transcript":transcript,"target_duration_seconds":target,"actual_duration_milliseconds":actual,"created_at":created});
                    if include_observations {
                        v["observations"] = json!(observation_json(&c, sid, Some(aid), None)?);
                    }
                    arr.push(v)
                }
                item["attempts"] = json!(arr);
            }
            if include_observations {
                item["session_observations"] = json!(observation_json(&c, sid, None, None)?);
            }
            result.push(item)
        }
        Ok(json!({"items":result}))
    }

    async fn record_practice_json(&self, payload: Value) -> Result<Value> {
        let x: RecordPracticeSessionRequest = serde_json::from_value(payload.clone())?;
        let request_hash = hash_payload(&payload)?;
        let mut c = self.conn()?;
        let tx = c.transaction()?;
        if let Some((sid,hash,replayable,response))=tx.query_row("SELECT session_id,request_hash,replayable,response_json FROM recorded_requests WHERE idempotency_key=?1",[&x.idempotency_key],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).optional()?{if replayable==0{bail!("legacy idempotency record cannot be replayed")};if hash!=request_hash{bail!("idempotency_key was already used with a different payload")};let mut out:Value=serde_json::from_str(&response)?;out["recorded"]=json!(false);out["idempotent_replay"]=json!(true);out["session_id"]=json!(sid);return Ok(out)}
        evidence::validate_session_observation_numbers(&x)?;
        let cfg = scheduler_config(&tx)?;
        let (reviewed_at, review_dt) = normalized_timestamp(x.reviewed_at.as_deref())?;
        let review_day = learning_day(review_dt, &cfg.timezone, cfg.cutoff)?;
        let today = learning_day(now_utc(), &cfg.timezone, cfg.cutoff)?;
        let date = x
            .session_date
            .clone()
            .unwrap_or_else(|| review_day.to_string());
        let session_day = valid_date(&date, "session_date")?;
        if x.reviewed_at.is_none() && session_day != today {
            bail!("backdated or future session_date requires reviewed_at")
        }
        if x.reviewed_at.is_some() && session_day != review_day {
            bail!("session_date must match the learning day of reviewed_at")
        }
        let declared = x
            .items
            .iter()
            .flat_map(|i| i.target_weakness_keys.iter().cloned())
            .collect::<HashSet<_>>();
        let mut created = Vec::new();
        let mut new_keys = HashSet::new();
        for w in &x.new_weaknesses {
            if !new_keys.insert(w.key.clone()) {
                bail!("duplicate new weakness key: {}", w.key)
            };
            let (_, was_created) = upsert_weakness_tx(&tx, w, declared.contains(&w.key), true)?;
            if was_created {
                created.push(w.key.clone())
            }
        }
        for key in all_weakness_keys(&x) {
            if !weakness_exists(&tx, &key)? {
                bail!("unknown weakness key: {key}")
            }
        }
        let (type_key, type_label) = resolve_exercise_type(
            &tx,
            x.exercise_type_key
                .as_deref()
                .or(Some(x.exercise_type.as_str())),
        )?;
        tx.execute("INSERT INTO sessions(session_date,exercise_type,exercise_type_key,topic,notes) VALUES(?1,?2,?3,?4,?5)",params![date,type_label,type_key,x.topic,x.notes])?;
        let sid = tx.last_insert_rowid();
        let mut item_ids = HashMap::new();
        for item in &x.items {
            tx.execute("INSERT INTO practice_items(session_id,item_no,drill_type,prompt,prompt_fingerprint,response,corrected_response,reference_answer,feedback,outcome) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![sid,item.item_no,item.drill_type.as_str(),item.prompt,prompt_fingerprint(&item.prompt),item.response,item.corrected_response,item.reference_answer,item.feedback,item.outcome.as_ref().map(serialize_enum).transpose()?])?;
            let iid = tx.last_insert_rowid();
            item_ids.insert(item.item_no, iid);
            for key in &item.target_weakness_keys {
                promote_target_if_needed(&tx, key)?;
                tx.execute("INSERT INTO practice_item_targets(practice_item_id,weakness_id) SELECT ?1,id FROM weaknesses WHERE key=?2",params![iid,key])?;
            }
        }
        let mut observation_ids = HashMap::new();
        for a in &x.attempts {
            let iid = a.practice_item_no.and_then(|n| item_ids.get(&n)).copied();
            tx.execute("INSERT INTO attempts(session_id,attempt_no,practice_item_id,transcript,target_duration_seconds,actual_duration_milliseconds) VALUES(?1,?2,?3,?4,?5,?6)",params![sid,a.attempt_no,iid,a.transcript,a.target_duration_seconds,a.actual_duration_milliseconds.map(i64::try_from).transpose().map_err(|_|anyhow!("actual duration overflow"))?])?;
            let aid = tx.last_insert_rowid();
            for o in &a.observations {
                insert_observation(&tx, sid, Some(aid), iid, o, &mut observation_ids)?;
            }
        }
        for item in &x.items {
            for o in &item.observations {
                insert_observation(
                    &tx,
                    sid,
                    None,
                    item_ids.get(&item.item_no).copied(),
                    o,
                    &mut observation_ids,
                )?;
            }
        }
        for o in &x.observations {
            insert_observation(&tx, sid, None, None, o, &mut observation_ids)?;
        }
        for key in all_weakness_keys(&x) {
            tx.execute("UPDATE weaknesses SET first_seen=min(COALESCE(first_seen,?2),?2),last_seen=max(COALESCE(last_seen,?2),?2) WHERE key=?1",params![key,date])?;
        }
        let mut review_keys = HashSet::new();
        let mut review_updates = Vec::new();
        for review in &x.reviews {
            if !review_keys.insert(review.weakness_key.clone()) {
                bail!("only one FSRS review per weakness is allowed in a session")
            };
            let row = load_one_weakness_tx(&tx, &review.weakness_key)?
                .ok_or_else(|| anyhow!("unknown weakness key: {}", review.weakness_key))?;
            if !has_target(&tx, sid, &review.weakness_key) {
                bail!(
                    "FSRS review must target a declared practice item: {}",
                    review.weakness_key
                )
            };
            let initial = validate_review_evidence(&tx, sid, &row, review, &observation_ids)?;
            let prior = match (row.stability, row.difficulty, row.last_review.as_deref()) {
                (Some(s), Some(d), Some(_)) => Some(fsrs_adapter::memory_state(s, d)?),
                (None, None, None) => None,
                _ => bail!("scheduler item has incomplete state"),
            };
            let elapsed = match row.last_review.as_deref() {
                Some(last) => learning_day_delta(last, &reviewed_at, &cfg.timezone, cfg.cutoff)?,
                None => 0,
            };
            let before = prior
                .map(|p| fsrs_adapter::retrievability(p, elapsed, &cfg.parameters))
                .transpose()?
                .map(f64::from);
            let scheduled = fsrs_adapter::schedule(
                prior,
                elapsed,
                review.rating.number(),
                cfg.desired_retention,
                &cfg.parameters,
            )?;
            let due = review_day + Duration::days(i64::from(scheduled.interval_days));
            let evidence = if review.evidence.is_null() {
                json!({})
            } else {
                review.evidence.clone()
            };
            let evidence_json = serde_json::to_string(&evidence)?;
            tx.execute("INSERT INTO weakness_reviews(scheduler_item_id,session_id,reviewed_at,review_learning_day,rating,rating_source,rating_rationale,retrieval_mode,evidence_strength,evidence_json,elapsed_days,desired_retention,retrievability_before,stability_before,difficulty_before,stability_after,difficulty_after,scheduled_interval_days,due_learning_day,algorithm,algorithm_version,parameter_set_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",params![row.scheduler_id, sid,reviewed_at,review_day.to_string(),review.rating.number(),review.rating_source.as_str(),review.rating_rationale,review.retrieval_mode.as_str(),serialize_enum(&review.evidence_strength)?,evidence_json,elapsed,cfg.desired_retention,before,prior.map(|p|f64::from(p.stability)),prior.map(|p|f64::from(p.difficulty)),f64::from(scheduled.memory.stability),f64::from(scheduled.memory.difficulty),scheduled.interval_days,due.to_string(),cfg.algorithm,cfg.algorithm_version,cfg.parameter_set_id])?;
            let rid = tx.last_insert_rowid();
            for (no, role) in review_evidence_roles(&tx, sid, review, &initial, &observation_ids)? {
                tx.execute("INSERT INTO review_observations(review_id,observation_id,evidence_role) VALUES(?1,?2,?3)",params![rid,observation_ids[&no],role])?;
            }
            tx.execute("UPDATE scheduler_items SET stability=?1,difficulty=?2,due_learning_day=?3,last_review_at=?4,algorithm_version=?5,parameter_set_id=?6 WHERE id=?7",params![f64::from(scheduled.memory.stability),f64::from(scheduled.memory.difficulty),due.to_string(),reviewed_at,cfg.algorithm_version,cfg.parameter_set_id,row.scheduler_id])?;
            review_updates.push(json!({"weakness_key":review.weakness_key,"rating":rating_name(review.rating.number()),"retrievability_before":before,"stability_before":prior.map(|p|f64::from(p.stability)),"stability_after":f64::from(scheduled.memory.stability),"difficulty_before":prior.map(|p|f64::from(p.difficulty)),"difficulty_after":f64::from(scheduled.memory.difficulty),"scheduled_interval_days":scheduled.interval_days,"due_learning_day":due.to_string()}));
        }
        let response = json!({"session_id":sid,"recorded":true,"idempotent_replay":false,"exercise_type_key":type_key,"exercise_type_label":type_label,"item_count":x.items.len(),"attempt_count":x.attempts.len(),"observation_count":observation_ids.len(),"new_weaknesses_created":created,"review_updates":review_updates});
        tx.execute("INSERT INTO recorded_requests(idempotency_key,session_id,request_hash,response_json,replayable) VALUES(?1,?2,?3,?4,1)",params![x.idempotency_key,sid,request_hash,serde_json::to_string(&response)?])?;
        tx.commit()?;
        Ok(response)
    }

    async fn review_queue(&self, request: ReviewQueueRequest) -> Result<Value> {
        let c = self.conn()?;
        let cfg = scheduler_config(&c)?;
        let day = request
            .as_of
            .as_deref()
            .map(|x| valid_date(x, "as_of"))
            .transpose()?
            .unwrap_or(learning_day(now_utc(), &cfg.timezone, cfg.cutoff)?);
        let cats = request.category.map(|x| vec![x]);
        let mut rows = load_weaknesses(
            &c,
            cats.as_deref(),
            None,
            request.concept_keys.as_deref(),
            request.scheme_keys.as_deref(),
            request.collection_keys.as_deref(),
            request.target_types.as_deref(),
            false,
        )?;
        sort_rows(&mut rows, day, &cfg)?;
        let include_upcoming = request.include_upcoming.unwrap_or(false);
        if !include_upcoming {
            rows.retain(|r| is_due(r, day))
        }
        let n = i32::from(request.limit.unwrap_or(20));
        rows.truncate(n.clamp(1, 50) as usize);
        let items=rows.iter().map(|r|Ok(json!({"key":r.key,"category":r.category,"description":r.description,"target_pattern":r.target_pattern,"classification":classification(&c,r.id,false)?,"due_learning_day":r.due,"is_due":is_due(r,day),"status":if r.due.is_none(){"new"}else if is_due(r,day){"due"}else{"upcoming"},"days_overdue":r.due.as_ref().and_then(|d|valid_date(d,"due_learning_day").ok()).map(|d|(day-d).num_days().max(0)).unwrap_or(0),"retrievability":row_retrievability(r,day,&cfg)?,"stability":r.stability,"difficulty":r.difficulty,"last_reviewed_at":r.last_review,"last_rating":r.last_rating.map(rating_name),"review_count":r.review_count,"legacy":{"due_date":r.legacy_due,"interval_days":r.legacy_interval,"ease_factor":r.legacy_ease,"repetitions":r.legacy_repetitions,"lapses":r.legacy_lapses}}))).collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"basis":"FSRS due date followed by deterministic historical onboarding priority for uninitialized targets","as_of":day.to_string(),"includes_upcoming":include_upcoming,"items":items}),
        )
    }

    async fn practice_brief(&self, request: PracticeBriefRequest) -> Result<Value> {
        let c = self.conn()?;
        let cfg = scheduler_config(&c)?;
        let mix = request.drill_mix.unwrap_or(DrillMix::Auto);
        let count = if matches!(mix, DrillMix::Fluency) {
            1
        } else {
            usize::from(request.count.unwrap_or(6))
        };
        let day = request
            .as_of
            .as_deref()
            .map(|x| valid_date(x, "as_of"))
            .transpose()?
            .unwrap_or(learning_day(now_utc(), &cfg.timezone, cfg.cutoff)?);
        let explicit = request.weakness_keys.as_deref();
        let mut rows = load_weaknesses(
            &c,
            request.categories.as_deref(),
            explicit,
            request.concept_keys.as_deref(),
            request.scheme_keys.as_deref(),
            request.collection_keys.as_deref(),
            request.target_types.as_deref(),
            false,
        )?;
        if let Some(keys) = explicit {
            let present = rows.iter().map(|r| r.key.as_str()).collect::<HashSet<_>>();
            for key in keys {
                if !present.contains(key.as_str()) {
                    bail!("unknown or inactive weakness key: {key}")
                }
            }
        }
        sort_rows(&mut rows, day, &cfg)?;
        let explicit_cap = explicit.is_some();
        let mut ordered = rows;
        if !explicit_cap {
            let mut due = ordered
                .iter()
                .filter(|r| r.due.is_some() && is_due(r, day))
                .cloned()
                .collect::<Vec<_>>();
            let mut new = ordered
                .iter()
                .filter(|r| r.due.is_none())
                .cloned()
                .collect::<Vec<_>>();
            new.truncate(2);
            let upcoming = ordered
                .iter()
                .filter(|r| r.due.is_some() && !is_due(r, day))
                .cloned()
                .collect::<Vec<_>>();
            let mut merged = Vec::new();
            if !matches!(request.objective, Some(PracticeObjective::Explore)) {
                merged.append(&mut due);
                merged.append(&mut new)
            } else {
                merged.append(&mut new);
                merged.append(&mut due)
            }
            if request.include_upcoming.unwrap_or(true) {
                merged.extend(upcoming)
            }
            ordered = merged;
        }
        diversify_rows(&mut ordered);
        let target_count = if explicit_cap {
            ordered.len().min(count)
        } else {
            ordered.len().min(count.div_ceil(2).max(1))
        };
        ordered.truncate(target_count);
        let allocations = if target_count == 0 {
            vec![]
        } else {
            allocate(count, target_count)
        };
        let prompt_limit = i32::from(request.recent_prompts_per_weakness.unwrap_or(4).min(10));
        let mut targets = Vec::new();
        for (r, allocation) in ordered.iter().zip(allocations) {
            let ret = row_retrievability(r, day, &cfg)?;
            let reason = if r.pending {
                "classification pending".to_owned()
            } else if r.due.is_none() {
                format!(
                    "uninitialized; errors on {} distinct historical days",
                    r.error_days
                )
            } else if is_due(r, day) {
                format!(
                    "due FSRS review; predicted retrievability {}",
                    ret.map(|x| format!("{x:.2}"))
                        .unwrap_or_else(|| "unknown".to_owned())
                )
            } else {
                "upcoming review".to_owned()
            };
            targets.push(json!({"weakness_key":r.key,"category":r.category,"description":r.description,"target_pattern":r.target_pattern,"classification":classification(&c,r.id,matches!(request.drill_mix,Some(DrillMix::Custom)))?,"allocation":allocation,"selection_reason":reason,"fsrs":fsrs_json(r,day,&cfg)?,"drill_recommendations":recommendations(&c,r.id)?,"recommended_drill_types":recommendations(&c,r.id)?.iter().filter_map(|v|v["drill_type"].as_str()).collect::<Vec<_>>(),"recent_prompts_to_avoid":fetch_recent_prompts(&c,r.id,prompt_limit)?,"recent_error_examples":recent_errors(&c,r.id)?}));
        }
        let total = targets
            .iter()
            .map(|v| v["allocation"].as_u64().unwrap_or(0))
            .sum::<u64>();
        Ok(
            json!({"as_of":day.to_string(),"objective":request.objective.unwrap_or(PracticeObjective::ReviewDue),"count":total,"level":request.level.unwrap_or_else(||"A2".to_owned()),"drill_mix":mix,"targets":targets,"generation_rules":[format!("Generate exactly {total} exercises using the allocations above"),"Use at least two materially varied cues when a target receives an FSRS rating","Do not reveal weakness keys or answers before the learner responds","Assess and record each item separately"]}),
        )
    }

    async fn upsert_weakness_json(&self, payload: Value) -> Result<Value> {
        let x: UpsertWeaknessRequest = serde_json::from_value(payload)?;
        let mut c = self.conn()?;
        let tx = c.transaction()?;
        let (id, created) = upsert_weakness_tx(
            &tx,
            &NewWeaknessInput {
                key: x.key.clone(),
                category: x.category,
                description: x.description,
                target_pattern: x.target_pattern,
                recommended_drill_types: x.recommended_drill_types,
                active: x.active,
                target_type: x.target_type,
                primary_concept_key: x.primary_concept_key,
                concept_links: x.concept_links,
                target_relations: x.target_relations,
            },
            false,
            false,
        )?;
        let row = classification(&tx, id, true)?;
        tx.commit()?;
        Ok(
            json!({"weakness_id":id,"key":x.key,"created":created,"updated":!created,"classification":row}),
        )
    }

    async fn taxonomy(&self, request: TaxonomyRequest) -> Result<Value> {
        let c = self.conn()?;
        get_taxonomy(&c, request)
    }
    async fn upsert_concept_json(&self, payload: Value) -> Result<Value> {
        let x: UpsertConceptRequest = serde_json::from_value(payload)?;
        let mut c = self.conn()?;
        let tx = c.transaction()?;
        let id = upsert_concept_tx(&tx, &x)?;
        let out = get_one_concept(&tx, id, x.include_full())?;
        tx.commit()?;
        Ok(out)
    }
    async fn data_status(&self) -> Result<Value> {
        let c = self.conn()?;
        let cfg = scheduler_config(&c)?;
        let count = |table: &str| -> Result<i64> {
            Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
        };
        Ok(
            json!({"schema_version":7,"storage_model":"single_learner","spaced_repetition":true,"scheduler":{"algorithm":cfg.algorithm,"algorithm_version":cfg.algorithm_version,"desired_retention":cfg.desired_retention,"parameter_set_id":cfg.parameter_set_id,"parameters_source":cfg.source},"last_session_date":c.query_row("SELECT max(session_date) FROM sessions",[],|r|r.get::<_,Option<String>>(0))?,"counts":{"sessions":count("sessions")?,"practice_items":count("practice_items")?,"attempts":count("attempts")?,"observations":count("observations")?,"weaknesses":count("weaknesses")?,"active_weaknesses":c.query_row("SELECT count(*) FROM weaknesses WHERE target_status='active'",[],|r|r.get::<_,i64>(0))?,"scheduler_items":count("scheduler_items")?,"weakness_reviews":count("weakness_reviews")?,"review_observations":count("review_observations")?}}),
        )
    }
    async fn ping(&self) -> Result<()> {
        self.conn()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }
}

fn resolve_exercise_type(c: &Connection, key: Option<&str>) -> Result<(String, String)> {
    let key = key.ok_or_else(|| anyhow!("exercise_type_key is required for new sessions"))?;
    let canonical = match key {
        "4-3-2" => "fluency_4_3_2",
        "production drill" => "production_drill",
        "translation_drill" => "translation_drill",
        "DELE A2 oral microdrill" => "dele_a2_oral_microdrill",
        "agreement/disagreement drill" => "agreement_disagreement_drill",
        "guided conversation drill" => "guided_conversation",
        x => x,
    };
    c.query_row(
        "SELECT key,label FROM exercise_types WHERE key=?1 AND active=1",
        [canonical],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .ok_or_else(|| anyhow!("unknown exercise_type_key: {canonical}"))
}
fn weakness_exists(c: &Connection, key: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM weaknesses WHERE key=?1)",
        [key],
        |r| r.get(0),
    )?)
}
fn load_one_weakness_tx(c: &Connection, key: &str) -> Result<Option<WeaknessRow>> {
    Ok(load_weaknesses(
        c,
        None,
        Some(&[key.to_owned()]),
        None,
        None,
        None,
        None,
        true,
    )?
    .into_iter()
    .next())
}
fn promote_target_if_needed(c: &Connection, key: &str) -> Result<()> {
    let status: Option<String> = c
        .query_row(
            "SELECT target_status FROM weaknesses WHERE key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    if status.as_deref() == Some("candidate") {
        c.execute(
            "UPDATE weaknesses SET target_status='active',active=1 WHERE key=?1",
            [key],
        )?;
        let id: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [key], |r| {
            r.get(0)
        })?;
        c.execute("INSERT OR IGNORE INTO scheduler_items(weakness_id,track,active) VALUES(?1,'general',1)",[id])?;
    }
    Ok(())
}
fn has_target(c: &Connection, sid: i64, key: &str) -> bool {
    c.query_row("SELECT EXISTS(SELECT 1 FROM practice_item_targets t JOIN practice_items p ON p.id=t.practice_item_id JOIN weaknesses w ON w.id=t.weakness_id WHERE p.session_id=?1 AND w.key=?2)",params![sid,key],|r|r.get(0)).unwrap_or(false)
}
fn all_weakness_keys(x: &RecordPracticeSessionRequest) -> Vec<String> {
    let mut v = Vec::new();
    v.extend(x.new_weaknesses.iter().map(|x| x.key.clone()));
    v.extend(
        x.items
            .iter()
            .flat_map(|i| i.target_weakness_keys.iter().cloned()),
    );
    v.extend(
        x.items
            .iter()
            .flat_map(|i| i.observations.iter().map(|o| o.weakness_key.clone())),
    );
    v.extend(
        x.attempts
            .iter()
            .flat_map(|a| a.observations.iter().map(|o| o.weakness_key.clone())),
    );
    v.extend(x.observations.iter().map(|o| o.weakness_key.clone()));
    v
}
fn insert_observation(
    c: &Connection,
    sid: i64,
    aid: Option<i64>,
    iid: Option<i64>,
    o: &ObservationInput,
    map: &mut HashMap<u16, i64>,
) -> Result<()> {
    let phase = evidence::normalized_phase(o)?;
    let hint = o.hint_level.unwrap_or(HintLevel::None);
    let no = o
        .observation_no
        .ok_or_else(|| anyhow!("observation_no is required"))?;
    let outcome = serialize_enum(&o.outcome)?;
    let evidence_strength = o
        .evidence_strength
        .map(|value| serialize_enum(&value))
        .transpose()?;
    let severity = o.severity.map(|value| serialize_enum(&value)).transpose()?;
    let changed=c.execute("INSERT INTO observations(session_id,attempt_id,practice_item_id,weakness_id,observation_no,outcome,role,assessment_phase,hint_level,learner_effort,evidence_source,evidence_strength,severity,produced,correction,error_span,notes) SELECT ?1,?2,?3,id,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16 FROM weaknesses WHERE key=?17",params![sid,aid,iid,no,outcome,serialize_enum(&o.role)?,phase.as_str(),hint.as_str(),o.learner_effort.map(|x|x.as_str()),o.evidence_source.unwrap_or(EvidenceSource::Assistant).as_str(),evidence_strength,severity,o.produced,o.correction,o.error_span,o.notes,o.weakness_key])?;
    if changed != 1 {
        bail!("unknown weakness key: {}", o.weakness_key)
    };
    map.insert(no, c.last_insert_rowid());
    Ok(())
}

fn upsert_weakness_tx(
    c: &Connection,
    w: &NewWeaknessInput,
    declared_item: bool,
    default_candidate: bool,
) -> Result<(i64, bool)> {
    let existing: Option<(i64, String, String, i64)> = c
        .query_row(
            "SELECT id,target_status,target_type,classification_pending FROM weaknesses WHERE key=?1",
            [&w.key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let was_created = existing.is_none();
    let status = match w.active {
        Some(true) => "active",
        Some(false) => "suspended",
        None => {
            if existing.is_some() {
                if let Some(existing) = existing.as_ref() {
                    existing.1.as_str()
                } else {
                    unreachable!()
                }
            } else if declared_item || !default_candidate {
                "active"
            } else {
                "candidate"
            }
        }
    }
    .to_owned();
    let typ = w
        .target_type
        .map(target_type_str)
        .map(str::to_owned)
        .or_else(|| existing.as_ref().map(|x| x.2.clone()))
        .unwrap_or_else(|| "grammatical_construction".to_owned());
    let drills = w
        .recommended_drill_types
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let classification_supplied = w.primary_concept_key.is_some() || !w.concept_links.is_empty();
    let classification_pending = if classification_supplied {
        0
    } else {
        existing.as_ref().map_or(1, |x| x.3)
    };
    c.execute("INSERT INTO weaknesses(key,category,description,target_pattern,recommended_drill_types,target_type,target_status,classification_pending,active) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(key) DO UPDATE SET category=excluded.category,description=excluded.description,target_pattern=excluded.target_pattern,recommended_drill_types=COALESCE(excluded.recommended_drill_types,weaknesses.recommended_drill_types),target_type=excluded.target_type,target_status=?7,classification_pending=?8,active=?9",params![w.key,w.category,w.description,w.target_pattern,drills,typ,status,classification_pending,taxonomy::active_flag_for_status(&status)?])?;
    let id = existing
        .as_ref()
        .map(|x| x.0)
        .unwrap_or_else(|| c.last_insert_rowid());
    if classification_supplied || was_created {
        c.execute("DELETE FROM weakness_concepts WHERE weakness_id=?1", [id])?;
        let primary = w
            .primary_concept_key
            .clone()
            .unwrap_or_else(|| unclassified_concept(&typ));
        ensure_concept(c, &primary)?;
        c.execute("INSERT INTO weakness_concepts(weakness_id,concept_id,role) SELECT ?1,id,'primary' FROM concepts WHERE key=?2",params![id,primary])?;
        for link in &w.concept_links {
            let key = link.concept_key.as_str();
            ensure_concept(c, key)?;
            c.execute("INSERT INTO weakness_concepts(weakness_id,concept_id,role) SELECT ?1,id,?2 FROM concepts WHERE key=?3",params![id,link.role.as_str(),key])?;
        }
    }
    for relation in &w.target_relations {
        insert_target_relation(c, &w.key, relation)?;
    }
    if status == "active" {
        c.execute("INSERT OR IGNORE INTO scheduler_items(weakness_id,track,active) VALUES(?1,'general',1)",[id])?;
    } else {
        c.execute(
            "UPDATE scheduler_items SET active=0 WHERE weakness_id=?1",
            [id],
        )?;
    }
    taxonomy::validate_primary_links(c)?;
    Ok((id, was_created))
}
fn unclassified_concept(typ: &str) -> String {
    match typ {
        "pronunciation" => "form.pronunciation",
        "orthography" => "form.orthography",
        "discourse_strategy" | "sociopragmatic_choice" => "use.unclassified",
        _ => "form.unclassified",
    }
    .to_owned()
}
fn ensure_concept(c: &Connection, key: &str) -> Result<()> {
    taxonomy::validate_concept_key(key)?;
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM concepts WHERE key=?1)",
        [key],
        |r| r.get(0),
    )? {
        bail!("unknown concept key: {key}")
    }
    Ok(())
}
fn insert_target_relation(c: &Connection, key: &str, r: &TargetRelationInput) -> Result<()> {
    taxonomy::validate_target_relation_predicate(&r.predicate)?;
    let other = &r.other_weakness_key;
    let (a, b) = if matches!(
        r.predicate.as_str(),
        "confusable_with" | "variant_of" | "practice_together"
    ) {
        taxonomy::canonical_pair(key, other)
    } else {
        (key, other.as_str())
    };
    let aid: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [a], |x| x.get(0))?;
    let bid: i64 = c.query_row("SELECT id FROM weaknesses WHERE key=?1", [b], |x| x.get(0))?;
    c.execute("INSERT OR IGNORE INTO weakness_relations(subject_id,predicate,object_id,notes) VALUES(?1,?2,?3,?4)",params![aid,r.predicate,bid,r.notes])?;
    Ok(())
}

fn validate_review_evidence(
    c: &Connection,
    sid: i64,
    row: &WeaknessRow,
    review: &crate::model::ReviewInput,
    map: &HashMap<u16, i64>,
) -> Result<u16> {
    if review.evidence_observation_nos.is_empty() {
        bail!("review must reference at least one observation")
    };
    if review
        .rating_rationale
        .as_ref()
        .is_some_and(|x| x.len() > 1000)
    {
        bail!("rating_rationale must be at most 1000 bytes")
    };
    if matches!(review.rating_source, RatingSource::AssistantSuggested)
        && review.rating == FsrsRating::Easy
    {
        bail!("easy requires learner confirmation")
    };
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for no in &review.evidence_observation_nos {
        if !seen.insert(*no) {
            bail!("review evidence observation numbers must be unique")
        };
        let id = *map
            .get(no)
            .ok_or_else(|| anyhow!("review references unknown observation_no {no}"))?;
        let rec:(i64,String,String,String,String,Option<String>,Option<String>,String)=c.query_row("SELECT weakness_id,role,assessment_phase,hint_level,outcome,evidence_strength,learner_effort,evidence_source FROM observations WHERE id=?1 AND session_id=?2",params![id,sid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?.ok_or_else(||anyhow!("review observation is not in the current session"))?;
        if rec.0 != row.id {
            bail!("review observations must have the same weakness")
        };
        records.push((*no, rec));
    }
    let cold = records
        .iter()
        .filter(|(_, r)| r.1 == "targeted" && r.2 == "cold_retrieval" && r.3 == "none")
        .collect::<Vec<_>>();
    let initial = cold
        .iter()
        .min_by_key(|(no, _)| *no)
        .ok_or_else(|| anyhow!("review requires a targeted cold retrieval with no hint"))?;
    if review.evidence_strength.as_str() != initial.1.5.as_deref().unwrap_or("") {
        bail!("review evidence_strength must match the initial cold observation")
    };
    if review.rating == FsrsRating::Easy
        && !(matches!(
            review.rating_source,
            RatingSource::Learner | RatingSource::AssistantSuggestedConfirmed
        ) && initial.1.6.as_deref() == Some("effortless"))
    {
        bail!("easy requires learner confirmation and effortless initial evidence")
    };
    if review.retrieval_mode == RetrievalMode::Recognition
        && review.evidence_strength != EvidenceStrength::Recognition
    {
        bail!("recognition retrieval must use recognition evidence strength")
    };
    Ok(initial.0)
}
fn review_evidence_roles(
    c: &Connection,
    sid: i64,
    review: &crate::model::ReviewInput,
    initial: &u16,
    map: &HashMap<u16, i64>,
) -> Result<Vec<(u16, &'static str)>> {
    review
        .evidence_observation_nos
        .iter()
        .map(|no| -> Result<(u16, &'static str)> {
            let role = if no == initial {
                "initial"
            } else {
                let phase: String = c.query_row(
                    "SELECT assessment_phase FROM observations WHERE id=?1 AND session_id=?2",
                    params![map[no], sid],
                    |r| r.get(0),
                )?;
                if phase == "transfer" {
                    "transfer"
                } else {
                    "supporting"
                }
            };
            Ok((*no, role))
        })
        .collect()
}

fn allocate(total: usize, len: usize) -> Vec<usize> {
    if len == 0 {
        return vec![];
    }
    let mut out = vec![1; len];
    let mut rest = total.saturating_sub(len);
    while rest > 0 {
        for x in &mut out {
            if rest == 0 {
                break;
            }
            *x += 1;
            rest -= 1;
        }
    }
    out
}

fn get_one_concept(c: &Connection, id: i64, full: bool) -> Result<Value> {
    let row:(String,String,String,Option<String>,Option<String>,i64)=c.query_row("SELECT c.key,c.label,cs.key,c.definition,c.source_uri,c.active FROM concepts c JOIN concept_schemes cs ON cs.id=c.scheme_id WHERE c.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let mut parents=c.prepare("SELECT p.key FROM concept_edges e JOIN concepts p ON p.id=e.object_id WHERE e.subject_id=?1 AND e.predicate='broader' ORDER BY p.key")?;
    let broader = parents
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let mut out = json!({"key":row.0,"label":row.1,"scheme_key":row.2,"active":row.5!=0,"broader_concept_keys":broader});
    if full {
        out["definition"] = json!(row.3);
        out["source_uri"] = json!(row.4);
    }
    Ok(out)
}
fn get_taxonomy(c: &Connection, request: TaxonomyRequest) -> Result<Value> {
    let depth = request.depth.unwrap_or(3);
    if !(1..=6).contains(&depth) {
        bail!("depth must be between 1 and 6")
    };
    let roots = request.root_concept_keys.unwrap_or_default();
    let ids = if request.include_descendants.unwrap_or(true) && !roots.is_empty() {
        taxonomy::descendant_ids(c, &roots, depth)?
    } else if !roots.is_empty() {
        roots
            .iter()
            .map(|x| taxonomy::concept_id(c, x))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let mut sql="SELECT c.id,c.key,c.label,cs.key,c.definition,c.source_uri,c.active FROM concepts c JOIN concept_schemes cs ON cs.id=c.scheme_id WHERE c.active=1".to_owned();
    let mut vals: Vec<SqlValue> = Vec::new();
    if let Some(s) = request.scheme_keys.as_ref().filter(|v| !v.is_empty()) {
        sql.push_str(" AND cs.key IN (");
        sql.push_str(
            &std::iter::repeat_n("?", s.len())
                .collect::<Vec<_>>()
                .join(","),
        );
        sql.push(')');
        vals.extend(s.iter().cloned().map(SqlValue::from));
    }
    if !ids.is_empty() {
        sql.push_str(" AND c.id IN (");
        sql.push_str(
            &std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(","),
        );
        sql.push(')');
        vals.extend(ids.iter().copied().map(SqlValue::from));
    }
    sql.push_str(" ORDER BY cs.key,c.sort_order,c.key LIMIT ?");
    let limit = i64::from(request.limit.unwrap_or(200).min(500));
    vals.push(limit.into());
    let mut st = c.prepare(&sql)?;
    let mut concepts = Vec::new();
    for r in st.query_map(rusqlite::params_from_iter(vals.iter()), |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
        ))
    })? {
        let (id, key, label, scheme, definition, source, active) = r?;
        let full = !request.compact.unwrap_or(false);
        let mut out = json!({"key":key,"label":label,"scheme_key":scheme,"active":active!=0});
        if full {
            out["definition"] = json!(definition);
            out["source_uri"] = json!(source);
        }
        let mut edge_query = c.prepare(
            "SELECT e.predicate, CASE WHEN e.subject_id=?1 THEN object.key ELSE subject.key END,
                    CASE WHEN e.subject_id=?1 THEN 'outgoing' ELSE 'incoming' END,
                    e.provenance, e.notes
             FROM concept_edges e
             JOIN concepts subject ON subject.id=e.subject_id
             JOIN concepts object ON object.id=e.object_id
             WHERE e.subject_id=?1 OR e.object_id=?1
             ORDER BY e.predicate, 2",
        )?;
        let edges = edge_query
            .query_map([id], |r| {
                Ok(json!({
                    "predicate": r.get::<_, String>(0)?,
                    "other_concept_key": r.get::<_, String>(1)?,
                    "direction": r.get::<_, String>(2)?,
                    "provenance": r.get::<_, String>(3)?,
                    "notes": if full { json!(r.get::<_, Option<String>>(4)?) } else { Value::Null }
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        out["edges"] = json!(edges);
        if request.include_targets.unwrap_or(false) {
            let mut q=c.prepare("SELECT w.key,w.target_type,w.target_status,wc.role FROM weakness_concepts wc JOIN weaknesses w ON w.id=wc.weakness_id WHERE wc.concept_id=?1 ORDER BY w.key LIMIT 100")?;
            out["targets"]=json!(q.query_map([id],|r|Ok(json!({"key":r.get::<_,String>(0)?,"target_type":r.get::<_,String>(1)?,"target_status":r.get::<_,String>(2)?,"role":r.get::<_,String>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        if request.include_collections.unwrap_or(false) {
            let mut q=c.prepare("SELECT co.key,co.label FROM collection_members cm JOIN collections co ON co.id=cm.collection_id JOIN weakness_concepts wc ON wc.weakness_id=cm.weakness_id WHERE wc.concept_id=?1 GROUP BY co.id ORDER BY co.key LIMIT 100")?;
            out["collections"] = json!(
                q.query_map([id], |r| Ok(
                    json!({"key":r.get::<_,String>(0)?,"label":r.get::<_,String>(1)?})
                ))?
                .collect::<rusqlite::Result<Vec<_>>>()?
            );
        }
        concepts.push(out)
    }
    Ok(
        json!({"schemes":request.scheme_keys.unwrap_or_default(),"roots":roots,"depth":depth,"include_descendants":request.include_descendants.unwrap_or(true),"concepts":concepts}),
    )
}

fn upsert_concept_tx(c: &Connection, x: &UpsertConceptRequest) -> Result<i64> {
    taxonomy::validate_concept_key(&x.key)?;
    ensure_scheme(c, &x.scheme_key)?;
    if x.label.trim().is_empty() || x.label.len() > 500 {
        bail!("concept label must be 1-500 bytes")
    };
    if x.edges.len() + x.broader_concept_keys.as_ref().map_or(0, Vec::len)
        > taxonomy::MAX_CONCEPT_EDGES_PER_REQUEST
    {
        bail!("at most 20 concept edges may be supplied")
    };
    let scheme: i64 = c.query_row(
        "SELECT id FROM concept_schemes WHERE key=?1",
        [&x.scheme_key],
        |r| r.get(0),
    )?;
    let id: i64 = match c
        .query_row("SELECT id FROM concepts WHERE key=?1", [&x.key], |r| {
            r.get(0)
        })
        .optional()?
    {
        Some(id) => {
            if x.active == Some(false) {
                taxonomy::ensure_concept_is_not_active_primary(c, id)?
            }
            c.execute("UPDATE concepts SET scheme_id=?1,label=?2,definition=?3,source_uri=?4,active=?5,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?6",params![scheme,x.label,x.definition,x.source_uri,x.active.map(i64::from).unwrap_or(1),id])?;
            id
        }
        None => {
            c.execute("INSERT INTO concepts(scheme_id,key,label,definition,source_uri,active) VALUES(?1,?2,?3,?4,?5,?6)",params![scheme,x.key,x.label,x.definition,x.source_uri,x.active.map(i64::from).unwrap_or(1)])?;
            c.last_insert_rowid()
        }
    };
    for parent in x.broader_concept_keys.iter().flatten() {
        ensure_concept(c, parent)?;
        insert_concept_edge_tx(c, &x.key, "broader", parent, "curated", None)?;
    }
    for edge in &x.edges {
        ensure_concept(c, &edge.other_concept_key)?;
        insert_concept_edge_tx(
            c,
            &x.key,
            &edge.predicate,
            &edge.other_concept_key,
            edge.provenance.as_deref().unwrap_or("curated"),
            edge.notes.as_deref(),
        )?;
    }
    Ok(id)
}
fn ensure_scheme(c: &Connection, key: &str) -> Result<()> {
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM concept_schemes WHERE key=?1 AND active=1)",
        [key],
        |r| r.get(0),
    )? {
        bail!("unknown or inactive concept scheme: {key}")
    }
    Ok(())
}
fn insert_concept_edge_tx(
    c: &Connection,
    a: &str,
    p: &str,
    b: &str,
    provenance: &str,
    notes: Option<&str>,
) -> Result<()> {
    taxonomy::validate_edge_predicate(p)?;
    if !matches!(provenance, "curated" | "external" | "inferred") {
        bail!("invalid concept edge provenance")
    };
    let (a, b) = if matches!(p, "contrasts_with" | "related") {
        taxonomy::canonical_pair(a, b)
    } else {
        (a, b)
    };
    let aid = taxonomy::concept_id(c, a)?;
    let bid = taxonomy::concept_id(c, b)?;
    if matches!(p, "broader" | "requires") {
        taxonomy::validate_edge_does_not_cycle(c, aid, p, bid)?;
    }
    c.execute("INSERT OR IGNORE INTO concept_edges(subject_id,predicate,object_id,provenance,notes) VALUES(?1,?2,?3,?4,?5)",params![aid,p,bid,provenance,notes])?;
    Ok(())
}

impl UpsertConceptRequest {
    fn include_full(&self) -> bool {
        true
    }
}

#[derive(Clone, Default)]
pub struct MockStore;
#[async_trait]
impl LearningStore for MockStore {
    async fn learning_context(&self, r: LearningContextRequest) -> Result<Value> {
        Ok(json!({"recent_sessions":r.recent_sessions.unwrap_or(5)}))
    }
    async fn recent_practice(&self, r: RecentPracticeRequest) -> Result<Value> {
        Ok(json!({"items":[],"limit":r.limit.unwrap_or(10),"skill":r.skill}))
    }
    async fn record_practice_json(&self, p: Value) -> Result<Value> {
        Ok(json!({"recorded":true,"payload":p}))
    }
    async fn review_queue(&self, r: ReviewQueueRequest) -> Result<Value> {
        Ok(
            json!({"items":[],"limit":r.limit.unwrap_or(20),"category":r.category,"as_of":r.as_of,"include_upcoming":r.include_upcoming.unwrap_or(false)}),
        )
    }
    async fn practice_brief(&self, _: PracticeBriefRequest) -> Result<Value> {
        Ok(json!({"targets":[]}))
    }
    async fn upsert_weakness_json(&self, p: Value) -> Result<Value> {
        Ok(json!({"updated":true,"patch":p}))
    }
    async fn taxonomy(&self, _: TaxonomyRequest) -> Result<Value> {
        Ok(json!({"concepts":[]}))
    }
    async fn upsert_concept_json(&self, p: Value) -> Result<Value> {
        Ok(json!({"updated":true,"patch":p}))
    }
    async fn data_status(&self) -> Result<Value> {
        Ok(json!({"schema_version":7,"status":"ready"}))
    }
    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}
pub type SharedStore = Arc<dyn LearningStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AssessmentPhase;
    fn test_store() -> SqliteStore {
        let p = std::env::temp_dir().join(format!("aprendiendo-{}.sqlite3", uuid::Uuid::new_v4()));
        SqliteStore::new(p).unwrap()
    }
    #[test]
    fn empty_store_has_seeded_taxonomy() {
        let s = test_store();
        let c = s.conn().unwrap();
        assert_eq!(
            c.query_row("SELECT count(*) FROM concepts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            46
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM scheduler_parameter_sets", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }
    #[test]
    fn timezone_uses_iana_dst() {
        let before = DateTime::parse_from_rfc3339("2026-03-29T00:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let after = DateTime::parse_from_rfc3339("2026-03-29T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            learning_day(before, "Europe/Madrid", 4).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 28).unwrap()
        );
        assert_eq!(
            learning_day(after, "Europe/Madrid", 4).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 28).unwrap()
        );
    }

    #[tokio::test]
    async fn deliberate_record_creates_one_audited_review_and_replays() {
        let store = test_store();
        let payload = json!({
            "idempotency_key": "test-deliberate-1",
            "exercise_type": "translation_drill",
            "new_weaknesses": [{
                "key": "future.lexical.target",
                "category": "lexis",
                "description": "A future lexical target",
                "target_pattern": "use the word",
                "target_type": "lexical_item"
            }],
            "items": [{
                "item_no": 1,
                "drill_type": "translation",
                "prompt": "Use the target.",
                "response": "Usé el objetivo.",
                "outcome": "correct",
                "target_weakness_keys": ["future.lexical.target"],
                "observations": [{
                    "observation_no": 1,
                    "weakness_key": "future.lexical.target",
                    "outcome": "correct",
                    "role": "targeted",
                    "assessment_phase": "cold_retrieval",
                    "hint_level": "none",
                    "evidence_strength": "controlled_production",
                    "learner_effort": "some_effort"
                }]
            }],
            "reviews": [{
                "weakness_key": "future.lexical.target",
                "rating": "good",
                "retrieval_mode": "controlled_production",
                "evidence_strength": "controlled_production",
                "evidence_observation_nos": [1]
            }]
        });
        let parsed: RecordPracticeSessionRequest = serde_json::from_value(payload.clone()).unwrap();
        assert_eq!(
            parsed.items[0].observations[0].assessment_phase,
            Some(AssessmentPhase::ColdRetrieval)
        );
        evidence::validate_session_observation_numbers(&parsed).unwrap();
        let first = store.record_practice_json(payload.clone()).await.unwrap();
        assert_eq!(first["recorded"], true);
        assert_eq!(first["review_updates"].as_array().unwrap().len(), 1);
        let scheduler_id: i64 = {
            let c = store.conn().unwrap();
            c.query_row(
                "SELECT si.id FROM scheduler_items si JOIN weaknesses w ON w.id=si.weakness_id WHERE w.key='future.lexical.target'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        let updated = store
            .upsert_weakness_json(json!({
                "key": "future.lexical.target",
                "category": "lexis",
                "description": "A classified lexical target",
                "target_pattern": "use the word",
                "primary_concept_key": "form.lexis.lexeme_choice"
            }))
            .await
            .unwrap();
        assert_eq!(
            updated["classification"]["primary_concept"]["key"],
            "form.lexis.lexeme_choice"
        );
        let replay = store.record_practice_json(payload).await.unwrap();
        assert_eq!(replay["idempotent_replay"], true);
        let c = store.conn().unwrap();
        assert_eq!(
            c.query_row(
                "SELECT target_type,classification_pending FROM weaknesses WHERE key='future.lexical.target'",
                [],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            ("lexical_item".to_owned(), 0)
        );
        assert_eq!(
            c.query_row(
                "SELECT si.id FROM scheduler_items si JOIN weaknesses w ON w.id=si.weakness_id WHERE w.key='future.lexical.target'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            scheduler_id
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM weakness_reviews", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM review_observations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM observations WHERE assessment_phase='cold_retrieval' AND observation_no=1",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
