//! SQLite-backed persistent directory history.
//!
//! This module owns durable directory records and the global event clock. It
//! deliberately does not inspect the filesystem, match queries, or rank
//! candidates; those policies belong to higher layers.

use std::{
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::frecency::{Record, first_visit};

const SCHEMA_VERSION: i32 = 1;
const TRACKING_BUSY_TIMEOUT: Duration = Duration::from_millis(100);
const ADMINISTRATIVE_BUSY_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_SQLITE_INTEGER: u64 = i64::MAX as u64;
const STATE_DIRECTORY: &str = "zfz";
const DATABASE_FILE_NAME: &str = "history.sqlite3";

/// A preserved directory path and the history used to rank it.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryRecord {
    /// Path text exactly as supplied by the caller.
    path: String,
    /// Incremental frecency state for `path`.
    history: Record,
}

impl DirectoryRecord {
    /// Returns the path text exactly as supplied by the caller.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the incremental frecency state used to rank this path.
    #[must_use]
    pub const fn history(&self) -> Record {
        self.history
    }

    /// Consumes this record, returning its preserved path and frecency state.
    #[must_use]
    pub fn into_parts(self) -> (String, Record) {
        (self.path, self.history)
    }
}

/// A consistent snapshot of the global clock and every stored directory.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    /// The latest recorded navigation event.
    tick: u64,
    /// Records sorted by their preserved path.
    records: Vec<DirectoryRecord>,
}

impl History {
    /// Returns the latest recorded navigation event.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Returns records sorted by their preserved path.
    #[must_use]
    pub fn records(&self) -> &[DirectoryRecord] {
        &self.records
    }

    /// Consumes this snapshot, returning its records in preserved-path order.
    #[must_use]
    pub fn into_records(self) -> Vec<DirectoryRecord> {
        self.records
    }

    const fn empty() -> Self {
        Self {
            tick: 0,
            records: Vec::new(),
        }
    }
}

/// Result of attempting an automatic tracking update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitOutcome {
    /// The visit was committed durably.
    Recorded,
    /// Another writer held the database beyond the tracking wait bound.
    Contended,
}

/// Failures reported by the persistent history layer.
#[derive(Debug)]
pub enum StorageError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// SQLite rejected an operation.
    Sql(rusqlite::Error),
    /// No usable XDG state directory can be derived from the environment.
    StateHomeUnavailable,
    /// The database uses a schema this program cannot safely interpret.
    UnsupportedSchema { found: i32 },
    /// Stored state or a caller-supplied path violates an invariant.
    InvalidData(String),
    /// The event clock or a visit count cannot be represented safely.
    CounterOverflow,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Sql(error) => error.fmt(formatter),
            Self::StateHomeUnavailable => {
                formatter.write_str("cannot determine XDG state home: set XDG_STATE_HOME or HOME")
            }
            Self::UnsupportedSchema { found } => write!(
                formatter,
                "history database schema version {found} is not supported"
            ),
            Self::InvalidData(message) => formatter.write_str(message),
            Self::CounterOverflow => formatter.write_str("history event counter overflow"),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Sql(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StorageError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

/// A SQLite history database at one explicit path.
#[derive(Debug, Clone)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    /// Creates a database handle without opening or creating the database.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the default XDG state-file location.
    pub fn default_path() -> Result<PathBuf, StorageError> {
        if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
            return Ok(PathBuf::from(state_home)
                .join(STATE_DIRECTORY)
                .join(DATABASE_FILE_NAME));
        }
        let home = env::var_os("HOME").ok_or(StorageError::StateHomeUnavailable)?;
        Ok(PathBuf::from(home)
            .join(".local/state")
            .join(STATE_DIRECTORY)
            .join(DATABASE_FILE_NAME))
    }

    /// Returns the filesystem location used by this handle.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads a consistent, read-only candidate snapshot.
    ///
    /// A database that has never been created behaves as an empty history and
    /// does not create state merely because it was read.
    pub fn load_candidates(&self) -> Result<History, StorageError> {
        if !self.path.exists() {
            return Ok(History::empty());
        }

        let mut connection =
            Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(TRACKING_BUSY_TIMEOUT)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        validate_schema(&transaction)?;
        let tick = read_tick(&transaction)?;
        let mut statement = transaction
            .prepare("SELECT path, visits, last_tick, score FROM records ORDER BY path")?;
        let records = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        transaction.commit()?;

        let records = records
            .into_iter()
            .map(|(path, visits, last_tick, score)| {
                let history = record_from_storage(&path, visits, last_tick, score, tick)?;
                Ok(DirectoryRecord { path, history })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(History { tick, records })
    }

    /// Records a visit, returning contention rather than delaying navigation.
    pub fn record_visit(&self, path: &str) -> Result<VisitOutcome, StorageError> {
        validate_path(path)?;
        match self.record_visit_inner(path, TRACKING_BUSY_TIMEOUT) {
            Err(error) if is_busy(&error) => Ok(VisitOutcome::Contended),
            result => result.map(|()| VisitOutcome::Recorded),
        }
    }

    /// Explicitly adds a path with the same semantics as a recorded visit.
    pub fn add(&self, path: &str) -> Result<(), StorageError> {
        validate_path(path)?;
        self.record_visit_inner(path, ADMINISTRATIVE_BUSY_TIMEOUT)
    }

    /// Removes exactly one preserved path, returning whether it existed.
    pub fn remove_exact(&self, path: &str) -> Result<bool, StorageError> {
        validate_path(path)?;
        if !self.path.exists() {
            return Ok(false);
        }
        let mut connection = self.open_for_write(ADMINISTRATIVE_BUSY_TIMEOUT)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = transaction.execute("DELETE FROM records WHERE path = ?1", [path])?;
        transaction.commit()?;
        Ok(removed != 0)
    }

    /// Removes a path and slash-boundary descendants, returning their count.
    pub fn remove_recursive(&self, path: &str) -> Result<usize, StorageError> {
        validate_path(path)?;
        if !self.path.exists() {
            return Ok(0);
        }
        let mut connection = self.open_for_write(ADMINISTRATIVE_BUSY_TIMEOUT)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let root = recursive_root(path);
        let removed = if root == "/" {
            transaction.execute("DELETE FROM records", [])?
        } else {
            let prefix = format!("{root}/");
            transaction.execute(
                "DELETE FROM records
                 WHERE path = ?1 OR substr(path, 1, length(?2)) = ?2",
                params![root, prefix],
            )?
        };
        transaction.commit()?;
        Ok(removed)
    }

    fn record_visit_inner(&self, path: &str, timeout: Duration) -> Result<(), StorageError> {
        let mut connection = self.open_for_write(timeout)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let tick = read_tick(&transaction)?;
        if tick >= MAX_SQLITE_INTEGER {
            return Err(StorageError::CounterOverflow);
        }
        let next_tick = tick + 1;
        let existing = transaction
            .query_row(
                "SELECT visits, last_tick, score FROM records WHERE path = ?1",
                [path],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                },
            )
            .optional()?;
        let history = match existing {
            Some((visits, last_tick, score)) => {
                let record = record_from_storage(path, visits, last_tick, score, tick)?;
                updated_record(record, next_tick)?
            }
            None => first_visit(next_tick),
        };
        transaction.execute(
            "UPDATE metadata SET tick = ?1 WHERE singleton = 1",
            [next_tick],
        )?;
        transaction.execute(
            "INSERT INTO records(path, visits, last_tick, score) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET visits = excluded.visits,
                 last_tick = excluded.last_tick, score = excluded.score",
            params![
                path,
                history.visits(),
                history.last_tick(),
                history.stored_score(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn open_for_write(&self, timeout: Duration) -> Result<Connection, StorageError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        connection.busy_timeout(timeout)?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        ensure_schema(&mut connection)?;
        Ok(connection)
    }
}

fn ensure_schema(connection: &mut Connection) -> Result<(), StorageError> {
    let version = schema_version(connection)?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version != 0 {
        return Err(StorageError::UnsupportedSchema { found: version });
    }

    connection.pragma_update(None, "journal_mode", "DELETE")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = schema_version(&transaction)?;
    if version == SCHEMA_VERSION {
        transaction.commit()?;
        return Ok(());
    }
    if version != 0 {
        return Err(StorageError::UnsupportedSchema { found: version });
    }
    transaction.execute_batch(
        "CREATE TABLE metadata (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            tick INTEGER NOT NULL
         );
         INSERT INTO metadata(singleton, tick) VALUES (1, 0);
         CREATE TABLE records (
            path TEXT PRIMARY KEY,
            visits INTEGER NOT NULL,
            last_tick INTEGER NOT NULL,
            score REAL NOT NULL
         ) WITHOUT ROWID;
         PRAGMA user_version = 1;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn schema_version(connection: &Connection) -> Result<i32, StorageError> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(StorageError::from)
}

fn validate_schema(connection: &Connection) -> Result<(), StorageError> {
    let found = schema_version(connection)?;
    if found == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(StorageError::UnsupportedSchema { found })
    }
}

fn read_tick(connection: &Connection) -> Result<u64, StorageError> {
    Ok(
        connection.query_row("SELECT tick FROM metadata WHERE singleton = 1", [], |row| {
            row.get(0)
        })?,
    )
}

fn validate_path(path: &str) -> Result<(), StorageError> {
    if path.is_empty() {
        return Err(StorageError::InvalidData("path is empty".into()));
    }
    // Rust strings and SQLite TEXT can contain NUL, but shell paths cannot.
    if path.as_bytes().contains(&0) {
        return Err(StorageError::InvalidData("path contains NUL".into()));
    }
    Ok(())
}

fn record_from_storage(
    path: &str,
    visits: u64,
    last_tick: u64,
    score: f64,
    tick: u64,
) -> Result<Record, StorageError> {
    validate_path(path)?;
    if visits == 0 {
        return Err(StorageError::InvalidData(format!("{path} has zero visits")));
    }
    if last_tick == 0 {
        return Err(StorageError::InvalidData(format!(
            "{path} has zero last tick"
        )));
    }
    if last_tick > tick {
        return Err(StorageError::InvalidData(format!(
            "{path} has last tick {last_tick} after global tick {tick}",
        )));
    }
    if !score.is_finite() {
        return Err(StorageError::InvalidData(
            "record score is not finite".into(),
        ));
    }
    if score < 0.0 {
        return Err(StorageError::InvalidData("record score is negative".into()));
    }
    Record::new(visits, last_tick, score)
        .map_err(|error| StorageError::InvalidData(format!("{path} has invalid frecency: {error}")))
}

fn updated_record(record: Record, next_tick: u64) -> Result<Record, StorageError> {
    // The decay formula belongs to frecency. This adapter enforces SQLite's
    // signed-integer boundary and keeps the validated read/update/write
    // sequence inside one transaction.
    if record.visits() >= MAX_SQLITE_INTEGER {
        return Err(StorageError::CounterOverflow);
    }
    let updated = record.visit(next_tick);
    if !updated.stored_score().is_finite() {
        return Err(StorageError::InvalidData(
            "updated frecency score is not finite".into(),
        ));
    }
    Ok(updated)
}

fn recursive_root(path: &str) -> &str {
    let root = path.trim_end_matches('/');
    if root.is_empty() { "/" } else { root }
}

fn is_busy(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Sql(rusqlite::Error::SqliteFailure(code, _))
            if matches!(code.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use pretty_assertions::assert_eq;
    use rusqlite::{Connection, TransactionBehavior};

    use super::{DATABASE_FILE_NAME, Database, History, StorageError, VisitOutcome};

    fn database(name: &str) -> Database {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Database::new(
            std::env::temp_dir()
                .join(format!("zfz-storage-{name}-{unique}"))
                .join(DATABASE_FILE_NAME),
        )
    }

    #[test]
    fn missing_database_loads_as_empty_without_creating_files() {
        let database = database("empty");
        assert_eq!(database.load_candidates().unwrap(), History::empty());
        assert!(!database.path().exists());
    }

    #[test]
    fn visits_preserve_paths_and_increment_history() {
        let database = database("visit");
        assert_eq!(
            database.record_visit("/symlink spelling/a path\n").unwrap(),
            VisitOutcome::Recorded
        );
        database.add("/symlink spelling/a path\n").unwrap();
        let history = database.load_candidates().unwrap();
        assert_eq!(history.tick(), 2);
        assert_eq!(history.records().len(), 1);
        assert_eq!(history.records()[0].path(), "/symlink spelling/a path\n");
        assert_eq!(history.records()[0].history().visits(), 2);
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn exact_and_recursive_removal_use_path_component_boundaries() {
        let database = database("remove");
        for path in ["/work", "/work/a", "/workspace", "/workshop/a"] {
            database.add(path).unwrap();
        }
        assert!(database.remove_exact("/work").unwrap());
        assert_eq!(database.remove_recursive("/work").unwrap(), 1);
        let paths: Vec<_> = database
            .load_candidates()
            .unwrap()
            .into_records()
            .into_iter()
            .map(|record| record.into_parts().0)
            .collect();
        assert_eq!(paths, ["/workshop/a", "/workspace"]);
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_paths_are_rejected_before_creating_a_database() {
        let database = database("nul");
        for path in ["", "bad\0path"] {
            assert!(matches!(
                database.add(path),
                Err(StorageError::InvalidData(_))
            ));
        }
        assert!(matches!(
            database.remove_recursive(""),
            Err(StorageError::InvalidData(_))
        ));
        assert!(!database.path().exists());
    }

    #[test]
    fn recursive_root_removal_clears_all_history() {
        let database = database("root-remove");
        for path in ["/", "/one", "/two/three"] {
            database.add(path).unwrap();
        }
        assert_eq!(database.remove_recursive("/").unwrap(), 3);
        let history = database.load_candidates().unwrap();
        assert_eq!(history.tick(), 3);
        assert!(history.records().is_empty());
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn recursive_removal_treats_a_trailing_slash_as_the_same_selector() {
        let database = database("trailing-slash");
        for path in ["/work", "/work/", "/work/child", "/workspace"] {
            database.add(path).unwrap();
        }
        assert_eq!(database.remove_recursive("/work/").unwrap(), 3);
        let paths: Vec<_> = database
            .load_candidates()
            .unwrap()
            .into_records()
            .into_iter()
            .map(|record| record.into_parts().0)
            .collect();
        assert_eq!(paths, ["/workspace"]);
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_writers_preserve_every_visit() {
        let database = database("concurrent");
        database.add("/initial").unwrap();
        let barrier = Arc::new(Barrier::new(9));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let database = database.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    database.add(&format!("/concurrent/{index}")).unwrap();
                })
            })
            .collect();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let history = database.load_candidates().unwrap();
        assert_eq!(history.tick(), 9);
        assert_eq!(history.records().len(), 9);
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn interrupted_transaction_rolls_back_cleanly() {
        let database = database("rollback");
        database.add("/committed").unwrap();
        {
            let mut connection = Connection::open(database.path()).unwrap();
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            transaction
                .execute("INSERT INTO records(path, visits, last_tick, score) VALUES ('/lost', 1, 2, 1.0)", [])
                .unwrap();
        }
        let history = database.load_candidates().unwrap();
        assert_eq!(history.tick(), 1);
        assert_eq!(history.records().len(), 1);
        assert_eq!(history.records()[0].path(), "/committed");
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn negative_persisted_integers_are_rejected() {
        let database = database("negative-integer");
        database.add("/corrupt").unwrap();
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute("UPDATE records SET visits = -1", [])
            .unwrap();
        assert!(matches!(
            database.load_candidates(),
            Err(StorageError::Sql(_))
        ));
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn impossible_persisted_records_are_rejected() {
        for (name, update) in [
            ("zero-visits", "UPDATE records SET visits = 0"),
            ("zero-last-tick", "UPDATE records SET last_tick = 0"),
            ("visits-after-tick", "UPDATE records SET visits = 2"),
            ("negative-score", "UPDATE records SET score = -0.1"),
            ("nonfinite-score", "UPDATE records SET score = 1e999"),
        ] {
            let database = database(name);
            database.add("/corrupt").unwrap();
            let connection = Connection::open(database.path()).unwrap();
            connection.execute(update, []).unwrap();
            assert!(matches!(
                database.load_candidates(),
                Err(StorageError::InvalidData(_))
            ));
            fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn sqlite_integer_limit_is_reported_as_counter_overflow() {
        let database = database("counter-overflow");
        database.add("/existing").unwrap();
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute("UPDATE metadata SET tick = ?1", [i64::MAX])
            .unwrap();
        assert!(matches!(
            database.add("/new"),
            Err(StorageError::CounterOverflow)
        ));
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }

    #[test]
    fn unsupported_schema_is_rejected_without_mutation() {
        let database = database("schema");
        database.add("/existing").unwrap();
        let connection = Connection::open(database.path()).unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            database.load_candidates(),
            Err(StorageError::UnsupportedSchema { found: 99 })
        ));
        assert!(matches!(
            database.add("/new"),
            Err(StorageError::UnsupportedSchema { found: 99 })
        ));
        fs::remove_dir_all(database.path().parent().unwrap()).unwrap();
    }
}
