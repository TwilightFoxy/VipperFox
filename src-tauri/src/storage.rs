use crate::models::{ActivityEntry, ActivityKind};
use rusqlite::{params, Connection};
use std::{path::Path, sync::Mutex};

const KEYRING_SERVICE: &str = "VipperFox";
const KEYRING_USER: &str = "twitch-access-token";

pub struct Storage {
    connection: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA busy_timeout=5000;
                 CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS last_messages (
                    channel_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    PRIMARY KEY(channel_id,user_id)
                 );
                 CREATE TABLE IF NOT EXISTS watch_streaks (channel_id TEXT NOT NULL,user_id TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(channel_id,user_id));
                 CREATE TABLE IF NOT EXISTS activities (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL,
                    title TEXT NOT NULL,
                    detail TEXT NOT NULL,
                    created_at TEXT NOT NULL
                 );",
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn record_message(&self, channel: &str, user: &str, timestamp: &str) -> Result<(), String> {
        self.connection.lock().map_err(|_| "Database lock failed")?
            .execute("INSERT INTO last_messages(channel_id,user_id,timestamp) VALUES(?1,?2,?3) ON CONFLICT(channel_id,user_id) DO UPDATE SET timestamp=MAX(last_messages.timestamp,excluded.timestamp)", params![channel,user,timestamp])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn last_messages(
        &self,
        channel: &str,
    ) -> Result<std::collections::HashMap<String, String>, String> {
        let connection = self.connection.lock().map_err(|_| "Database lock failed")?;
        let mut statement = connection
            .prepare("SELECT user_id,timestamp FROM last_messages WHERE channel_id=?1")
            .map_err(|error| error.to_string())?;
        let result = statement
            .query_map([channel], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string());
        result
    }

    pub fn set_message_date(
        &self,
        channel: &str,
        user: &str,
        timestamp: &str,
    ) -> Result<(), String> {
        self.connection.lock().map_err(|_| "Database lock failed")?
            .execute("INSERT INTO last_messages(channel_id,user_id,timestamp) VALUES(?1,?2,?3) ON CONFLICT(channel_id,user_id) DO UPDATE SET timestamp=excluded.timestamp", params![channel,user,timestamp]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn record_streak(&self, channel: &str, user: &str, count: u64) -> Result<(), String> {
        let count = i64::try_from(count).map_err(|_| "Некорректная серия")?;
        self.connection.lock().map_err(|_| "Database lock failed")?
            .execute("INSERT INTO watch_streaks(channel_id,user_id,count) VALUES(?1,?2,?3) ON CONFLICT(channel_id,user_id) DO UPDATE SET count=excluded.count", params![channel,user,count]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn streaks(&self, channel: &str) -> Result<std::collections::HashMap<String, u64>, String> {
        let connection = self.connection.lock().map_err(|_| "Database lock failed")?;
        let mut query = connection
            .prepare("SELECT user_id,count FROM watch_streaks WHERE channel_id=?1")
            .map_err(|e| e.to_string())?;
        let result = query
            .query_map([channel], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string());
        result
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, String> {
        let connection = self.connection.lock().map_err(|_| "Database lock failed")?;
        let mut statement = connection
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|error| error.to_string())?;
        match statement.query_row([key], |row| row.get(0)) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|_| "Database lock failed")?
            .execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|_| "Database lock failed")?
            .execute("DELETE FROM settings WHERE key=?1", [key])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn save_token(&self, token: &str) -> Result<(), String> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| error.to_string())?
            .set_password(token)
            .map_err(|error| format!("Не удалось сохранить токен в хранилище Windows: {error}"))
    }

    pub fn load_token(&self) -> Result<Option<String>, String> {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| error.to_string())?
            .get_password()
        {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn delete_token(&self) -> Result<(), String> {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|error| error.to_string())?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn add_activity(
        &self,
        kind: ActivityKind,
        title: &str,
        detail: &str,
    ) -> Result<ActivityEntry, String> {
        let created_at = chrono::Utc::now().to_rfc3339();
        let kind_text = match kind {
            ActivityKind::VipAdd => "vip_add",
            ActivityKind::VipRemove => "vip_remove",
            ActivityKind::System => "system",
            ActivityKind::Warning => "warning",
        };
        let connection = self.connection.lock().map_err(|_| "Database lock failed")?;
        connection
            .execute(
                "INSERT INTO activities(kind,title,detail,created_at) VALUES(?1,?2,?3,?4)",
                params![kind_text, title, detail, created_at],
            )
            .map_err(|error| error.to_string())?;
        // The visible history is intentionally bounded so a machine left running
        // for years cannot grow the database without limit.
        connection
            .execute(
                "DELETE FROM activities WHERE id NOT IN (SELECT id FROM activities ORDER BY id DESC LIMIT 2000)",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(ActivityEntry {
            id: connection.last_insert_rowid(),
            kind,
            title: title.to_owned(),
            detail: detail.to_owned(),
            created_at,
        })
    }

    pub fn activities(&self, limit: usize) -> Result<Vec<ActivityEntry>, String> {
        let connection = self.connection.lock().map_err(|_| "Database lock failed")?;
        let mut statement = connection
            .prepare(
                "SELECT id,kind,title,detail,created_at FROM activities ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                let kind: String = row.get(1)?;
                Ok(ActivityEntry {
                    id: row.get(0)?,
                    kind: match kind.as_str() {
                        "vip_add" => ActivityKind::VipAdd,
                        "vip_remove" => ActivityKind::VipRemove,
                        "warning" => ActivityKind::Warning,
                        _ => ActivityKind::System,
                    },
                    title: row.get(2)?,
                    detail: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_database_is_extended_without_losing_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO settings VALUES('streak_threshold','150');").unwrap();
        drop(old);
        let storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage.setting("streak_threshold").unwrap().as_deref(),
            Some("150")
        );
        storage
            .set_message_date("channel", "42", "2026-09-01T12:00:00+00:00")
            .unwrap();
        storage.record_streak("channel", "42", 150).unwrap();
        assert_eq!(storage.streaks("channel").unwrap()["42"], 150);
    }

    #[test]
    fn activity_survives_reopen_and_is_isolated_by_channel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        {
            let storage = Storage::open(&path).unwrap();
            storage.set_setting("client_id", "existing-client").unwrap();
            storage
                .record_message("channel-a", "viewer", "2026-09-27T12:00:00+00:00")
                .unwrap();
            storage
                .record_message("channel-a", "viewer", "2026-09-26T12:00:00+00:00")
                .unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage.last_messages("channel-a").unwrap()["viewer"],
            "2026-09-27T12:00:00+00:00"
        );
        assert!(storage.last_messages("channel-b").unwrap().is_empty());
        assert_eq!(
            storage.setting("client_id").unwrap().as_deref(),
            Some("existing-client")
        );
    }

    #[test]
    fn manual_date_and_streak_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.db");
        {
            let storage = Storage::open(&path).unwrap();
            storage
                .record_message("a", "42", "2026-09-27T12:00:00+00:00")
                .unwrap();
            storage
                .set_message_date("a", "42", "2026-09-25T12:00:00+00:00")
                .unwrap();
            storage.record_streak("a", "42", 150).unwrap();
            storage.record_streak("a", "42", 3).unwrap();
        }
        let storage = Storage::open(&path).unwrap();
        assert_eq!(
            storage.last_messages("a").unwrap()["42"],
            "2026-09-25T12:00:00+00:00"
        );
        assert_eq!(storage.streaks("a").unwrap()["42"], 3);
        assert!(storage.streaks("b").unwrap().is_empty());
    }

    #[test]
    fn settings_and_history_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::open(&directory.path().join("test.db")).unwrap();
        storage.set_setting("threshold", "150").unwrap();
        assert_eq!(
            storage.setting("threshold").unwrap().as_deref(),
            Some("150")
        );
        storage
            .add_activity(ActivityKind::System, "Ready", "Connected")
            .unwrap();
        assert_eq!(storage.activities(10).unwrap()[0].title, "Ready");
    }
}
