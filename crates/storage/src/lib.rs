mod migrations;
pub mod repository;

pub use repository::Repository;

use rusqlite::{Connection, Result as SqlResult};
use std::path::Path;
use tracing::info;

/// Open (or create) the agent's local SQLite database.
/// Applies all pending migrations and configures WAL mode + performance pragmas.
pub fn open(db_path: &Path) -> SqlResult<Connection> {
    let conn = Connection::open(db_path)?;
    configure_connection(&conn)?;
    migrations::apply_all(&conn)?;
    info!(path = %db_path.display(), "Database opened");
    Ok(conn)
}

/// In-memory DB for testing.
pub fn open_in_memory() -> SqlResult<Connection> {
    let conn = Connection::open_in_memory()?;
    configure_connection(&conn)?;
    migrations::apply_all(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> SqlResult<()> {
    // WAL mode: concurrent readers don't block writers
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    // Synchronous=NORMAL: good balance of durability and performance
    conn.execute_batch("PRAGMA synchronous = NORMAL;")?;
    // Limit page cache to ~4 MB (1000 pages × 4 KB)
    conn.execute_batch("PRAGMA cache_size = -4000;")?;
    // Enable foreign key enforcement
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(())
}
