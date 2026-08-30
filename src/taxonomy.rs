use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, params};

pub const MAX_CONCEPT_EDGES_PER_REQUEST: usize = 20;
pub const CONCEPT_KEY_MAX_BYTES: usize = 160;

pub fn validate_concept_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > CONCEPT_KEY_MAX_BYTES {
        bail!("concept key must be 1-{CONCEPT_KEY_MAX_BYTES} bytes");
    }
    let mut segment_is_empty = true;
    for byte in key.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            segment_is_empty = false;
        } else if matches!(byte, b'.' | b'_' | b'-') && !segment_is_empty {
            segment_is_empty = true;
        } else {
            bail!("concept key must match lower-case ASCII segments separated by . _ or -");
        }
    }
    if segment_is_empty {
        bail!("concept key must match lower-case ASCII segments separated by . _ or -");
    }
    Ok(())
}

pub fn canonical_pair<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

pub fn validate_edge_predicate(predicate: &str) -> Result<()> {
    if matches!(
        predicate,
        "broader" | "requires" | "contrasts_with" | "related"
    ) {
        Ok(())
    } else {
        bail!("unsupported concept edge predicate: {predicate}")
    }
}

pub fn validate_target_relation_predicate(predicate: &str) -> Result<()> {
    if matches!(
        predicate,
        "confusable_with" | "variant_of" | "supersedes" | "practice_together"
    ) {
        Ok(())
    } else {
        bail!("unsupported target relation predicate: {predicate}")
    }
}

pub fn active_flag_for_status(status: &str) -> Result<i64> {
    match status {
        "candidate" | "active" => Ok(1),
        "suspended" | "retired" | "merged" => Ok(0),
        _ => Err(anyhow!("unsupported target status: {status}")),
    }
}

pub fn validate_primary_links(c: &Connection) -> Result<()> {
    let invalid: i64 = c.query_row(
        "SELECT count(*) FROM weaknesses w
         WHERE w.target_status IN ('candidate','active')
           AND (SELECT count(*) FROM weakness_concepts wc
                WHERE wc.weakness_id=w.id AND wc.role='primary') <> 1",
        [],
        |r| r.get(0),
    )?;
    if invalid != 0 {
        bail!(
            "every candidate or active weakness must have exactly one primary concept ({invalid} invalid)"
        );
    }
    Ok(())
}

pub fn validate_edge_does_not_cycle(
    c: &Connection,
    subject_id: i64,
    predicate: &str,
    object_id: i64,
) -> Result<()> {
    validate_edge_predicate(predicate)?;
    if !matches!(predicate, "broader" | "requires") {
        return Ok(());
    }
    let cycle: bool = c.query_row(
        "WITH RECURSIVE reachable(id) AS (
           SELECT ?2
           UNION
           SELECT e.object_id FROM concept_edges e JOIN reachable r ON r.id=e.subject_id
           WHERE e.predicate=?3
         ) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?1)",
        params![subject_id, object_id, predicate],
        |r| r.get(0),
    )?;
    if cycle {
        bail!("concept edge would create a {predicate} cycle");
    }
    Ok(())
}

pub fn concept_id(c: &Connection, key: &str) -> Result<i64> {
    c.query_row("SELECT id FROM concepts WHERE key=?1", [key], |r| r.get(0))
        .map_err(Into::into)
}

pub fn concept_key(c: &Connection, id: i64) -> Result<String> {
    c.query_row("SELECT key FROM concepts WHERE id=?1", [id], |r| r.get(0))
        .map_err(Into::into)
}

pub fn descendant_ids(c: &Connection, roots: &[String], max_depth: u8) -> Result<Vec<i64>> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }
    if !(1..=6).contains(&max_depth) {
        bail!("taxonomy depth must be between 1 and 6");
    }
    let placeholders = std::iter::repeat_n("?", roots.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "WITH RECURSIVE tree(id, depth) AS (
           SELECT id, 0 FROM concepts WHERE key IN ({placeholders})
           UNION
           SELECT e.subject_id, tree.depth+1 FROM concept_edges e JOIN tree ON tree.id=e.object_id
           WHERE e.predicate='broader' AND tree.depth < ?
         ) SELECT DISTINCT id FROM tree ORDER BY id"
    );
    let mut values = roots.to_vec();
    values.push(max_depth.to_string());
    let mut stmt = c.prepare(&sql)?;
    Ok(stmt
        .query_map(rusqlite::params_from_iter(values.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?)
}

pub fn ensure_concept_is_not_active_primary(c: &Connection, concept_id: i64) -> Result<()> {
    let used: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM weakness_concepts wc JOIN weaknesses w ON w.id=wc.weakness_id
         WHERE wc.concept_id=?1 AND wc.role='primary' AND w.target_status IN ('candidate','active'))",
        [concept_id],
        |r| r.get(0),
    )?;
    if used {
        bail!("concept is the primary concept of a candidate or active target");
    }
    Ok(())
}
