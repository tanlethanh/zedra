//! Read-only access to third-party WAL-mode SQLite databases (Codex, OpenCode).
//!
//! SQLite WAL databases cannot be opened read-only unless one of:
//! - `-wal` and `-shm` sidecars already exist and are readable,
//! - the opener can create those sidecars (writable directory), or
//! - the connection uses `immutable=1` (read checkpointed main file only).
//!
//! See <https://www.sqlite.org/wal.html#readonly> and
//! <https://www.sqlite.org/uri.html#uriimmutable>.
//!
//! Callers must pick `immutable=1` when sidecars are absent. Opening with `mode=ro`
//! alone can fail (sqlite3 CLI) or create new sidecars in `~/.codex` (in-process).

use rusqlite::{types::Value, Connection, OpenFlags};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

pub fn wal_sidecar_paths(db_path: &Path) -> (PathBuf, PathBuf) {
    let base = db_path.as_os_str().to_string_lossy();
    (format!("{base}-wal").into(), format!("{base}-shm").into())
}

pub fn wal_sidecars_present(db_path: &Path) -> bool {
    let (wal, shm) = wal_sidecar_paths(db_path);
    wal.is_file() && shm.is_file()
}

pub fn readonly_uri(db_path: &Path) -> String {
    let abs = db_path
        .canonicalize()
        .unwrap_or_else(|_| db_path.to_path_buf());
    let mut uri = String::from("file://");
    for byte in abs.as_os_str().as_encoded_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b'~' => {
                uri.push(char::from(*byte));
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri.push_str("?mode=ro");
    if !wal_sidecars_present(&abs) {
        uri.push_str("&immutable=1");
    }
    uri
}

pub fn open(db_path: &Path) -> Result<Connection, String> {
    let uri = readonly_uri(db_path);
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| {
        format!(
            "failed to open sqlite database {}: {error}",
            db_path.display()
        )
    })
}

pub fn query_json(db_path: &Path, query: &str) -> Result<Vec<u8>, String> {
    let connection = open(db_path)?;
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("sqlite prepare failed: {error}"))?;
    let column_names = statement
        .column_names()
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let mut rows = statement
        .query([])
        .map_err(|error| format!("sqlite query failed: {error}"))?;
    let mut values = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("sqlite query failed: {error}"))?
    {
        let mut object = serde_json::Map::new();
        for (index, name) in column_names.iter().enumerate() {
            let value = row
                .get::<_, Value>(index)
                .map_err(|error| format!("sqlite row decode failed: {error}"))?;
            object.insert(name.clone(), value_to_json(value));
        }
        values.push(JsonValue::Object(object));
    }
    serde_json::to_vec(&values).map_err(|error| error.to_string())
}

fn value_to_json(value: Value) -> JsonValue {
    match value {
        Value::Null => JsonValue::Null,
        Value::Integer(int) => json!(int),
        Value::Real(real) => json!(real),
        Value::Text(text) => JsonValue::String(text),
        Value::Blob(_) => JsonValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn command_on_path(program: &str) -> bool {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
            .unwrap_or(false)
    }

    #[test]
    fn readonly_uri_uses_immutable_when_wal_sidecars_are_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.sqlite");
        let setup = Command::new("sqlite3")
            .arg(&db_path)
            .arg("PRAGMA journal_mode=WAL; CREATE TABLE t(id INTEGER);")
            .status()
            .expect("sqlite3 setup");
        assert!(setup.success());
        let (wal, shm) = wal_sidecar_paths(&db_path);
        std::fs::remove_file(wal).ok();
        std::fs::remove_file(shm).ok();

        assert!(!wal_sidecars_present(&db_path));
        assert!(readonly_uri(&db_path).contains("immutable=1"));
    }

    #[test]
    fn open_reads_wal_database_without_creating_sidecars() {
        if !command_on_path("sqlite3") {
            return;
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("state.sqlite");
        let setup = Command::new("sqlite3")
            .arg(&db_path)
            .arg(
                "PRAGMA journal_mode=WAL; \
                 CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL); \
                 INSERT INTO threads VALUES ('t1', '/repo');",
            )
            .status()
            .expect("sqlite3 setup");
        assert!(setup.success());
        let (wal, shm) = wal_sidecar_paths(&db_path);
        std::fs::remove_file(wal).ok();
        std::fs::remove_file(shm).ok();
        assert!(!wal_sidecars_present(&db_path));

        let count: i64 = open(&db_path)
            .expect("open")
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 1);
        assert!(!wal_sidecars_present(&db_path));
    }
}
