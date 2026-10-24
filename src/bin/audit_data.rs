use anyhow::{Context, Result, bail};
use aprendiendo_mcp::{audit, migrations};
use rusqlite::Connection;
use std::env;
use std::path::PathBuf;

fn value_after(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf> {
    value_after(args, name)
        .map(PathBuf::from)
        .with_context(|| format!("missing {name}"))
}

fn open_database(path: &PathBuf) -> Result<Connection> {
    let mut c =
        Connection::open(path).with_context(|| format!("open database {}", path.display()))?;
    migrations::run(&mut c)?;
    Ok(c)
}

fn usage() -> ! {
    eprintln!(
        "usage: audit_data <export|review|progress|validate|preview|apply|verify|revert> [options]"
    );
    eprintln!("  export  --db PATH --out DIR");
    eprintln!("  review  --db PATH --out DIR");
    eprintln!("  progress --db PATH [--as-of YYYY-MM-DD]");
    eprintln!("  plan-repairs --db PATH --out FILE");
    eprintln!("  validate|preview|apply --db PATH --batch FILE --batch-id ID");
    eprintln!("  verify  --db PATH [--batch-id ID]");
    eprintln!("  revert  --db PATH --batch-id ID [--snapshot-hash HASH]");
    std::process::exit(2)
}

fn main() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    let command = args.get(1).map(String::as_str).unwrap_or_else(|| usage());
    match command {
        "export" => {
            let db = required_path(&args, "--db")?;
            let out = required_path(&args, "--out")?;
            let manifest = audit::export_database(&db, &out)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        "plan-repairs" => {
            let db = required_path(&args, "--db")?;
            let out = required_path(&args, "--out")?;
            let c = open_database(&db)?;
            let batch = audit::known_repair_batch(&db, &c)?;
            std::fs::write(&out, serde_json::to_vec_pretty(&batch)?)
                .with_context(|| format!("write audit batch {}", out.display()))?;
            println!(
                "wrote {} proposals to {}",
                batch.proposals.len(),
                out.display()
            );
        }
        "review" => {
            let db = required_path(&args, "--db")?;
            let out = required_path(&args, "--out")?;
            let c = open_database(&db)?;
            let export_dir = out.join("export");
            let manifest = audit::export_database(&db, &export_dir)?;
            let batch = audit::known_repair_batch(&db, &c)?;
            let batch_path = out.join("proposals.json");
            std::fs::create_dir_all(&out)?;
            std::fs::write(&batch_path, serde_json::to_vec_pretty(&batch)?)?;
            audit::validate_batch(&c, &batch)?;
            let report = serde_json::json!({
                "status": "deterministic_source_review_complete",
                "provider": "none; source-verifiable operator review only",
                "snapshot_boundary": manifest.snapshot_boundary.clone(),
                "records_examined": manifest.counts.get("sessions").copied().unwrap_or(0),
                "proposed_count": batch.proposals.len(),
                "proposals": batch_path.display().to_string(),
                "export": export_dir.display().to_string(),
                "remaining_unavailable_sources": manifest.missing_sources.clone(),
                "effective_evidence_summary": audit::effective_evidence_summary(&c)?,
                "unresolved_cases": [
                    "session 41 exact five-point prompt checklist and original conversation export are unavailable",
                    "sessions 40-41 linguistic findings and the supermarket phrase require original context review",
                    "broad agreement cases remain unclassified where source context is insufficient"
                ],
                "latest_successful_boundary": manifest.snapshot_boundary,
                "apply_policy": "review proposals separately, then use audit_data apply with the matching batch ID; this command never mutates data"
            });
            std::fs::write(
                out.join("audit-report.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        "progress" => {
            let db = required_path(&args, "--db")?;
            let c = open_database(&db)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&audit::weekly_progress(
                    &c,
                    value_after(&args, "--as-of").as_deref()
                )?)?
            );
        }
        "validate" | "preview" | "apply" => {
            let db = required_path(&args, "--db")?;
            let batch_path = required_path(&args, "--batch")?;
            let batch_id = value_after(&args, "--batch-id").context("--batch-id is required")?;
            let batch = audit::read_batch(&batch_path)?;
            if batch.audit_run_id != batch_id {
                bail!("--batch-id does not match batch.audit_run_id")
            }
            let mut c = open_database(&db)?;
            match command {
                "validate" => {
                    audit::validate_batch(&c, &batch)?;
                    println!(
                        "audit batch {batch_id} is valid; {} proposals",
                        batch.proposals.len()
                    );
                }
                "preview" => println!(
                    "{}",
                    serde_json::to_string_pretty(&audit::preview_batch(&c, &batch)?)?
                ),
                "apply" => {
                    let (applied, skipped) = audit::apply_batch(&mut c, &batch)?;
                    println!(
                        "audit batch {batch_id} applied: {applied} changes, {skipped} already applied"
                    );
                }
                _ => unreachable!(),
            }
        }
        "verify" => {
            let db = required_path(&args, "--db")?;
            let c = open_database(&db)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&audit::verify_database(
                    &c,
                    value_after(&args, "--batch-id").as_deref()
                )?)?
            );
        }
        "revert" => {
            let db = required_path(&args, "--db")?;
            let batch_id = value_after(&args, "--batch-id").context("--batch-id is required")?;
            let snapshot_hash = value_after(&args, "--snapshot-hash")
                .unwrap_or_else(|| "revert-source-not-recorded".into());
            let mut c = open_database(&db)?;
            let (new_id, count) = audit::revert_batch(&mut c, &batch_id, &snapshot_hash)?;
            println!("compensating audit batch {new_id} applied: {count} changes");
        }
        _ => usage(),
    }
    Ok(())
}
