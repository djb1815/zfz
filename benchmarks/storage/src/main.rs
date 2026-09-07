use std::{
    env, fs,
    path::Path,
    process::{Command, ExitCode},
    time::Instant,
};

#[cfg(feature = "sqlite")]
use zfz_storage_benchmark::sqlite::{JournalMode, SqliteStore};
use zfz_storage_benchmark::{Error, Store, Timings, dataset, journal::JournalStore, query};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments.len() < 3 {
        return Err(Error::Invalid(usage().into()));
    }
    let command = arguments[0].as_str();
    let backend = arguments[1].as_str();
    let root = Path::new(&arguments[2]);
    let store = store(backend, root)?;
    let total = Instant::now();
    let mut timings = Timings::default();

    match command {
        "init" => {
            let count = parse_count(arguments.get(3))?;
            store.initialise(&dataset::generate(count))?;
        }
        "query" => {
            let state = store.load(&mut timings)?;
            let started = Instant::now();
            let terms: Vec<&str> = arguments[3..].iter().map(String::as_str).collect();
            if let Some(path) = query(&state, &terms)? {
                println!("{path}");
            }
            timings.match_rank = started.elapsed();
        }
        "update" => {
            let path = arguments
                .get(3)
                .ok_or_else(|| Error::Invalid("update requires a path".into()))?;
            store.update(path, &mut timings)?;
        }
        "burst" => {
            let count = parse_count(arguments.get(3))?;
            let working_set = parse_count(arguments.get(4))?;
            spawn_burst(backend, root, count, working_set)?;
        }
        "prepare-replay" => {
            let records = parse_count(arguments.get(3))?;
            let entries = parse_count(arguments.get(4))?;
            store.initialise(&dataset::generate(records))?;
            spawn_burst(backend, root, entries, 128)?;
        }
        "copy" => {
            let destination = arguments
                .get(3)
                .ok_or_else(|| Error::Invalid("copy requires a destination".into()))?;
            copy_store(backend, root, Path::new(destination))?;
        }
        "compact" => match backend {
            "journal" => JournalStore::new(root).compact(&mut timings)?,
            #[cfg(feature = "sqlite")]
            "sqlite-delete" => SqliteStore::new(root, JournalMode::Delete).compact(&mut timings)?,
            #[cfg(feature = "sqlite")]
            "sqlite-wal" => SqliteStore::new(root, JournalMode::Wal).compact(&mut timings)?,
            _ => unreachable!("backend was already validated"),
        },
        "verify" => {
            let state = store.verify()?;
            println!("{}\t{}", state.tick, state.records.len());
        }
        "bytes" => println!("{}", store.bytes()?),
        _ => return Err(Error::Invalid(usage().into())),
    }

    if env::var_os("ZFZ_TIMINGS").is_some() {
        eprintln!(
            "timings_ns\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            timings.open.as_nanos(),
            timings.load.as_nanos(),
            timings.decode.as_nanos(),
            timings.match_rank.as_nanos(),
            timings.encode.as_nanos(),
            timings.commit.as_nanos(),
            timings.replace.as_nanos(),
            total.elapsed().as_nanos(),
        );
    }
    Ok(())
}

fn store(backend: &str, root: &Path) -> Result<Box<dyn Store>, Error> {
    match backend {
        "journal" => Ok(Box::new(JournalStore::new(root))),
        #[cfg(feature = "sqlite")]
        "sqlite-delete" => Ok(Box::new(SqliteStore::new(root, JournalMode::Delete))),
        #[cfg(feature = "sqlite")]
        "sqlite-wal" => Ok(Box::new(SqliteStore::new(root, JournalMode::Wal))),
        _ => Err(Error::Invalid(format!("unknown backend {backend:?}"))),
    }
}

fn parse_count(value: Option<&String>) -> Result<usize, Error> {
    value
        .ok_or_else(|| Error::Invalid("init requires a record count".into()))?
        .parse()
        .map_err(|_| Error::Invalid("record count must be an integer".into()))
}

fn spawn_burst(backend: &str, root: &Path, count: usize, working_set: usize) -> Result<(), Error> {
    if working_set == 0 {
        return Err(Error::Invalid("burst working set must be nonzero".into()));
    }
    let executable = env::current_exe()?;
    for index in 0..count {
        let path = format!("/benchmark/burst/{}", index % working_set);
        let status = Command::new(&executable)
            .args(["update", backend, root.to_str().unwrap(), &path])
            .status()?;
        if !status.success() {
            return Err(Error::Invalid("burst child failed".into()));
        }
    }
    Ok(())
}

fn copy_store(backend: &str, source: &Path, destination: &Path) -> Result<(), Error> {
    fs::create_dir_all(destination)?;
    let names: &[&str] = match backend {
        "journal" => &["snapshot", "journal", "lock", "snapshot.new"],
        "sqlite-delete" | "sqlite-wal" => &[
            "history.sqlite3",
            "history.sqlite3-journal",
            "history.sqlite3-wal",
            "history.sqlite3-shm",
        ],
        _ => return Err(Error::Invalid(format!("unknown backend {backend:?}"))),
    };
    for name in names {
        let target = destination.join(name);
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let original = source.join(name);
        if original.exists() {
            fs::copy(original, target)?;
        }
    }
    Ok(())
}

fn usage() -> &'static str {
    "usage: storage-benchmark <init|query|update|burst|prepare-replay|copy|compact|verify|bytes> \
     <journal|sqlite-delete|sqlite-wal> <store-directory> [argument]"
}
