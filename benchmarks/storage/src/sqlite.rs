use std::{fs, path::PathBuf, time::Instant};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{
    DirectoryRecord, Error, State, Store, Timings, directory_bytes, first_visit, record_from_parts,
    stored_score, visit,
};

const PROTOTYPE_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const PRODUCTION_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    Rowid,
    WithoutRowid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionSetup {
    Prototype,
    ProductionLike,
}

#[derive(Debug, Clone)]
pub struct SqliteStore {
    root: PathBuf,
    mode: JournalMode,
    schema: Schema,
    setup: ConnectionSetup,
}

impl SqliteStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, mode: JournalMode) -> Self {
        Self {
            root: root.into(),
            mode,
            schema: Schema::Rowid,
            setup: ConnectionSetup::Prototype,
        }
    }

    #[must_use]
    pub fn without_rowid(root: impl Into<PathBuf>, mode: JournalMode) -> Self {
        Self {
            root: root.into(),
            mode,
            schema: Schema::WithoutRowid,
            setup: ConnectionSetup::Prototype,
        }
    }

    #[must_use]
    pub fn production_like(root: impl Into<PathBuf>, mode: JournalMode) -> Self {
        Self {
            root: root.into(),
            mode,
            schema: Schema::WithoutRowid,
            setup: ConnectionSetup::ProductionLike,
        }
    }

    fn connect_for_initialisation(&self) -> Result<Connection, Error> {
        fs::create_dir_all(&self.root)?;
        let connection = Connection::open_with_flags(
            self.root.join("history.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        connection.busy_timeout(PROTOTYPE_BUSY_TIMEOUT)?;
        connection.pragma_update(None, "journal_mode", self.mode.sql())?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    fn connect_for_read(&self) -> Result<Connection, Error> {
        if self.setup == ConnectionSetup::Prototype {
            return self.connect_for_initialisation();
        }
        let connection = Connection::open_with_flags(
            self.root.join("history.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        connection.busy_timeout(PRODUCTION_BUSY_TIMEOUT)?;
        Ok(connection)
    }

    fn connect_for_write(&self) -> Result<Connection, Error> {
        if self.setup == ConnectionSetup::Prototype {
            return self.connect_for_initialisation();
        }
        let connection = Connection::open_with_flags(
            self.root.join("history.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        connection.busy_timeout(PRODUCTION_BUSY_TIMEOUT)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    fn create_schema(&self, connection: &Connection) -> Result<(), Error> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                tick INTEGER NOT NULL
             );",
        )?;
        let suffix = match self.schema {
            Schema::Rowid => "",
            Schema::WithoutRowid => " WITHOUT ROWID",
        };
        connection.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS records (
                path TEXT PRIMARY KEY,
                visits INTEGER NOT NULL,
                last_tick INTEGER NOT NULL,
                score REAL NOT NULL
             ){suffix};"
        ))?;
        Ok(())
    }

    pub fn compact(&self, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let connection = self.connect_for_write()?;
        timings.open += open_started.elapsed();
        let commit_started = Instant::now();
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        timings.commit += commit_started.elapsed();
        Ok(())
    }

    pub fn vacuum(&self, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let connection = self.connect_for_write()?;
        timings.open += open_started.elapsed();
        let commit_started = Instant::now();
        connection.execute_batch("VACUUM")?;
        timings.commit += commit_started.elapsed();
        Ok(())
    }
}

impl Store for SqliteStore {
    fn initialise(&self, state: &State) -> Result<(), Error> {
        state.validate()?;
        let mut connection = self.connect_for_initialisation()?;
        self.create_schema(&connection)?;
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
                    record.history.visits(),
                    record.history.last_tick(),
                    stored_score(record.history)
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn load(&self, timings: &mut Timings) -> Result<State, Error> {
        let open_started = Instant::now();
        let mut connection = self.connect_for_read()?;
        timings.open += open_started.elapsed();
        let load_started = Instant::now();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let tick: u64 =
            transaction.query_row("SELECT tick FROM metadata WHERE singleton = 1", [], |row| {
                row.get(0)
            })?;
        let mut statement = transaction
            .prepare("SELECT path, visits, last_tick, score FROM records ORDER BY path")?;
        let persisted_records = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let records = persisted_records
            .into_iter()
            .map(|(path, visits, last_tick, score)| {
                Ok(DirectoryRecord {
                    path,
                    history: record_from_parts(visits, last_tick, score)?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        drop(statement);
        transaction.commit()?;
        timings.load += load_started.elapsed();
        let state = State { tick, records };
        state.validate()?;
        Ok(state)
    }

    fn update(&self, path: &str, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let mut connection = self.connect_for_write()?;
        if self.setup == ConnectionSetup::Prototype {
            self.create_schema(&connection)?;
        }
        timings.open += open_started.elapsed();
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tick: u64 = transaction.query_row(
            "UPDATE metadata SET tick = tick + 1 WHERE singleton = 1 RETURNING tick",
            [],
            |row| row.get(0),
        )?;
        let persisted_existing = transaction
            .query_row(
                "SELECT visits, last_tick, score FROM records WHERE path = ?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let existing = persisted_existing
            .map(|(visits, last_tick, score)| record_from_parts(visits, last_tick, score))
            .transpose()?;
        let history = match existing {
            Some(record) => visit(record, tick)?,
            None => first_visit(tick)?,
        };
        transaction.execute(
            "INSERT INTO records(path, visits, last_tick, score) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET visits=excluded.visits,
             last_tick=excluded.last_tick, score=excluded.score",
            params![
                path,
                history.visits(),
                history.last_tick(),
                stored_score(history)
            ],
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
        let connection = self.connect_for_read()?;
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

    #[test]
    fn rowid_and_without_rowid_schemas_preserve_equivalent_state() {
        for (name, store) in [
            (
                "rowid",
                SqliteStore::new(
                    std::env::temp_dir().join(format!("zfz-sqlite-rowid-{}", std::process::id())),
                    JournalMode::Delete,
                ),
            ),
            (
                "without-rowid",
                SqliteStore::without_rowid(
                    std::env::temp_dir()
                        .join(format!("zfz-sqlite-without-rowid-{}", std::process::id())),
                    JournalMode::Delete,
                ),
            ),
            (
                "production-like",
                SqliteStore::production_like(
                    std::env::temp_dir()
                        .join(format!("zfz-sqlite-production-like-{}", std::process::id())),
                    JournalMode::Delete,
                ),
            ),
        ] {
            let _ = fs::remove_dir_all(&store.root);
            let initial = generate(100);
            store.initialise(&initial).unwrap();
            store
                .update("/new path/naïve", &mut Timings::default())
                .unwrap();
            let state = store.verify().unwrap();
            assert_eq!(state.tick, initial.tick + 1, "{name}");
            assert_eq!(state.records.len(), 101, "{name}");
            fs::remove_dir_all(&store.root).unwrap();
        }
    }
}
