use anyhow::{Context, Result};
use aprendiendo_mcp::migrations;
use rusqlite::Connection;
use std::{env, path::PathBuf};

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| env::var_os("DATABASE_PATH").map(PathBuf::from))
        .context("usage: migrate <sqlite-path> (or set DATABASE_PATH)")?;
    let mut connection = Connection::open(&path)
        .with_context(|| format!("open SQLite database {}", path.display()))?;
    migrations::run(&mut connection)?;
    if env::args().any(|a| a == "--seed-spontaneous-preferences") {
        aprendiendo_mcp::production::seed(&connection)?;
        println!("approved practice preferences seeded (existing edits preserved)");
    }
    println!("migration complete: {}", path.display());
    Ok(())
}
