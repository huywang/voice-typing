use log::{error, info};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// A single history record for a voice transcription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    pub id: i64,
    pub timestamp: String,
    pub raw_text: String,
    pub polished_text: String,
    pub duration_secs: f64,
    pub app_name: String,
}

/// Thread-safe wrapper around an SQLite connection for history storage.
pub struct HistoryDb {
    conn: Mutex<Connection>,
}

impl HistoryDb {
    /// Open (or create) the history database at the given app data directory.
    pub fn init(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("history.db");
        info!("Opening history database at {:?}", db_path);

        let conn = Connection::open(&db_path).map_err(|e| {
            error!("Failed to open history database: {e}");
            format!("Failed to open history database: {e}")
        })?;

        // Run migration
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp     TEXT NOT NULL,
                raw_text      TEXT NOT NULL,
                polished_text TEXT NOT NULL,
                duration_secs REAL NOT NULL,
                app_name      TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_history_timestamp ON history(timestamp DESC);",
        )
        .map_err(|e| {
            error!("Failed to run history migration: {e}");
            format!("Failed to run history migration: {e}")
        })?;

        info!("History database initialized");
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Insert a new history record. Returns the row id.
    pub fn insert(&self, record: &HistoryRecord) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO history (timestamp, raw_text, polished_text, duration_secs, app_name)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.timestamp,
                record.raw_text,
                record.polished_text,
                record.duration_secs,
                record.app_name,
            ],
        )
        .map_err(|e| {
            error!("Failed to insert history record: {e}");
            format!("Failed to insert history record: {e}")
        })?;
        Ok(conn.last_insert_rowid())
    }

    /// List history records with pagination, newest first.
    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<HistoryRecord>, String> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, raw_text, polished_text, duration_secs, app_name
                 FROM history ORDER BY id DESC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| format!("Failed to prepare list query: {e}"))?;

        let rows = stmt
            .query_map(params![limit, offset], |row| {
                Ok(HistoryRecord {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    raw_text: row.get(2)?,
                    polished_text: row.get(3)?,
                    duration_secs: row.get(4)?,
                    app_name: row.get(5)?,
                })
            })
            .map_err(|e| format!("Failed to query history: {e}"))?;

        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|e| format!("Failed to read history row: {e}"))?);
        }
        Ok(records)
    }

    /// Delete all history records.
    pub fn clear(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM history", [])
            .map_err(|e| {
                error!("Failed to clear history: {e}");
                format!("Failed to clear history: {e}")
            })?;
        info!("History cleared");
        Ok(())
    }

    /// Return the total number of history records.
    pub fn count(&self) -> Result<u32, String> {
        let conn = self.conn.lock().unwrap();
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .map_err(|e| format!("Failed to count history: {e}"))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn create_test_db() -> (HistoryDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = HistoryDb::init(dir.path()).unwrap();
        (db, dir)
    }

    fn sample_record() -> HistoryRecord {
        HistoryRecord {
            id: 0,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            raw_text: "hello world".to_string(),
            polished_text: "Hello, world!".to_string(),
            duration_secs: 2.5,
            app_name: "".to_string(),
        }
    }

    #[test]
    fn test_insert_and_list() {
        let (db, _dir) = create_test_db();
        let record = sample_record();
        let id = db.insert(&record).unwrap();
        assert!(id > 0);

        let records = db.list(10, 0).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].raw_text, "hello world");
        assert_eq!(records[0].polished_text, "Hello, world!");
    }

    #[test]
    fn test_count() {
        let (db, _dir) = create_test_db();
        assert_eq!(db.count().unwrap(), 0);

        db.insert(&sample_record()).unwrap();
        assert_eq!(db.count().unwrap(), 1);

        db.insert(&sample_record()).unwrap();
        assert_eq!(db.count().unwrap(), 2);
    }

    #[test]
    fn test_clear() {
        let (db, _dir) = create_test_db();
        db.insert(&sample_record()).unwrap();
        db.insert(&sample_record()).unwrap();
        assert_eq!(db.count().unwrap(), 2);

        db.clear().unwrap();
        assert_eq!(db.count().unwrap(), 0);
    }

    #[test]
    fn test_pagination() {
        let (db, _dir) = create_test_db();
        for i in 0..5 {
            let mut r = sample_record();
            r.raw_text = format!("record {i}");
            db.insert(&r).unwrap();
        }

        let page1 = db.list(2, 0).unwrap();
        assert_eq!(page1.len(), 2);
        // Newest first
        assert_eq!(page1[0].raw_text, "record 4");
        assert_eq!(page1[1].raw_text, "record 3");

        let page2 = db.list(2, 2).unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].raw_text, "record 2");
    }
}
