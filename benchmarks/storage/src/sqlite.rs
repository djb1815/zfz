use std::{fs, path::PathBuf, time::Instant};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{DirectoryRecord, Error, State, Store, Timings, directory_bytes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalMode {
    Delete,
    Wal,
}

impl JournalMode {
    fn sql(self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::Wal => "WAL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SqliteStore {
    root: PathBuf,
    mode: JournalMode,
}

impl SqliteStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, mode: JournalMode) -> Self {
        Self {
            root: root.into(),
            mode,
        }
    }

    fn connect(&self) -> Result<Connection, Error> {
        fs::create_dir_all(&self.root)?;
        let connection = Connection::open_with_flags(
            self.root.join("history.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(30))?;
        connection.pragma_update(None, "journal_mode", self.mode.sql())?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    fn create_schema(connection: &Connection) -> Result<(), Error> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                tick INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS records (
                path TEXT PRIMARY KEY,
                visits INTEGER NOT NULL,
                last_tick INTEGER NOT NULL,
                score REAL NOT NULL
             );",
        )?;
        Ok(())
    }

    pub fn compact(&self, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let connection = self.connect()?;
        timings.open += open_started.elapsed();
        let commit_started = Instant::now();
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        timings.commit += commit_started.elapsed();
        Ok(())
    }
}

impl Store for SqliteStore {
    fn initialise(&self, state: &State) -> Result<(), Error> {
        state.validate()?;
        let mut connection = self.connect()?;
        Self::create_schema(&connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM records", [])?;
        transaction.execute("DELETE FROM metadata", [])?;
        transaction.execute(
            "INSERT INTO metadata(singleton, tick) VALUES (1, ?1)",
            [state.tick],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO records(path, visits, last_tick, score) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for record in &state.records {
                statement.execute(params![
                    record.path,
                    record.history.visits,
                    record.history.last_tick,
                    record.history.score
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn load(&self, timings: &mut Timings) -> Result<State, Error> {
        let open_started = Instant::now();
        let mut connection = self.connect()?;
        timings.open += open_started.elapsed();
        let load_started = Instant::now();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let tick: u64 =
            transaction.query_row("SELECT tick FROM metadata WHERE singleton = 1", [], |row| {
                row.get(0)
            })?;
        let mut statement = transaction
            .prepare("SELECT path, visits, last_tick, score FROM records ORDER BY path")?;
        let records = statement
            .query_map([], |row| {
                Ok(DirectoryRecord {
                    path: row.get(0)?,
                    history: zfz::frecency::Record {
                        visits: row.get(1)?,
                        last_tick: row.get(2)?,
                        score: row.get(3)?,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        transaction.commit()?;
        timings.load += load_started.elapsed();
        let state = State { tick, records };
        state.validate()?;
        Ok(state)
    }

    fn update(&self, path: &str, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let mut connection = self.connect()?;
        Self::create_schema(&connection)?;
        timings.open += open_started.elapsed();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tick: u64 = transaction.query_row(
            "UPDATE metadata SET tick = tick + 1 WHERE singleton = 1 RETURNING tick",
            [],
            |row| row.get(0),
        )?;
        let existing = transaction
            .query_row(
                "SELECT visits, last_tick, score FROM records WHERE path = ?1",
                [path],
                |row| {
                    Ok(zfz::frecency::Record {
                        visits: row.get(0)?,
                        last_tick: row.get(1)?,
                        score: row.get(2)?,
                    })
                },
            )
            .optional()?;
        let history = existing.map_or_else(
            || zfz::frecency::first_visit(tick),
            |record| record.visit(tick, zfz::frecency::DEFAULT_LAMBDA),
        );
        transaction.execute(
            "INSERT INTO records(path, visits, last_tick, score) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET visits=excluded.visits,
             last_tick=excluded.last_tick, score=excluded.score",
            params![path, history.visits, history.last_tick, history.score],
        )?;
        if std::env::var("ZFZ_FAULT").as_deref() == Ok("sqlite-before-commit") {
            std::process::abort();
        }
        let commit_started = Instant::now();
        transaction.commit()?;
        timings.commit += commit_started.elapsed();
        if std::env::var("ZFZ_FAULT").as_deref() == Ok("sqlite-after-commit") {
            std::process::abort();
        }
        Ok(())
    }

    fn verify(&self) -> Result<State, Error> {
        let connection = self.connect()?;
        let integrity: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(Error::Invalid(format!(
                "SQLite integrity check failed: {integrity}"
            )));
        }
        drop(connection);
        self.load(&mut Timings::default())
    }

    fn bytes(&self) -> Result<u64, Error> {
        directory_bytes(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{JournalMode, SqliteStore};
    use crate::{Store, Timings, dataset::generate};

    #[test]
    fn both_journal_modes_preserve_equivalent_state() {
        for mode in [JournalMode::Delete, JournalMode::Wal] {
            let root =
                std::env::temp_dir().join(format!("zfz-sqlite-{mode:?}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let store = SqliteStore::new(&root, mode);
            store.initialise(&generate(100)).unwrap();
            store
                .update("/new path/naïve", &mut Timings::default())
                .unwrap();
            let state = store.verify().unwrap();
            assert!(
                state
                    .records
                    .iter()
                    .any(|record| record.path == "/new path/naïve")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }
}
