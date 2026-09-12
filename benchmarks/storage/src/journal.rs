use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use crc32fast::hash;
use fs2::FileExt;

use crate::{DirectoryRecord, Error, State, Store, Timings, directory_bytes};

const SNAPSHOT_MAGIC: &[u8; 8] = b"ZFZSNP01";
const MAX_PATH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct JournalStore {
    root: PathBuf,
}

impl JournalStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn snapshot_path(&self) -> PathBuf {
        self.root.join("snapshot")
    }

    fn journal_path(&self) -> PathBuf {
        self.root.join("journal")
    }

    fn lock(&self, exclusive: bool) -> Result<File, Error> {
        fs::create_dir_all(&self.root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("lock"))?;
        if exclusive {
            file.lock_exclusive()?;
        } else {
            FileExt::lock_shared(&file)?;
        }
        Ok(file)
    }

    fn load_unlocked(&self, timings: &mut Timings) -> Result<(State, usize), Error> {
        let read_started = Instant::now();
        let snapshot = fs::read(self.snapshot_path())?;
        let journal = fs::read(self.journal_path()).unwrap_or_default();
        timings.load += read_started.elapsed();

        let decode_started = Instant::now();
        let mut state = decode_snapshot(&snapshot)?;
        let valid_journal_bytes = replay_journal(&mut state, &journal)?;
        state.validate()?;
        timings.decode += decode_started.elapsed();
        Ok((state, valid_journal_bytes))
    }

    pub fn compact(&self, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let _lock = self.lock(true)?;
        timings.open += open_started.elapsed();
        let (state, _) = self.load_unlocked(timings)?;
        fault("compact-loaded");

        let encode_started = Instant::now();
        let snapshot = encode_snapshot(&state)?;
        timings.encode += encode_started.elapsed();
        let (write_duration, replace_duration) =
            write_snapshot_bytes_atomic(&self.root, &snapshot)?;
        timings.commit += write_duration;
        timings.replace += replace_duration;
        fault("compact-snapshot-replaced");
        let commit_started = Instant::now();
        replace_file(&self.journal_path(), &[])?;
        sync_directory(&self.root)?;
        timings.commit += commit_started.elapsed();
        Ok(())
    }
}

impl Store for JournalStore {
    fn initialise(&self, state: &State) -> Result<(), Error> {
        state.validate()?;
        let _lock = self.lock(true)?;
        write_snapshot_bytes_atomic(&self.root, &encode_snapshot(state)?)?;
        replace_file(&self.journal_path(), &[])?;
        sync_directory(&self.root)?;
        Ok(())
    }

    fn load(&self, timings: &mut Timings) -> Result<State, Error> {
        let open_started = Instant::now();
        let _lock = self.lock(false)?;
        timings.open += open_started.elapsed();
        self.load_unlocked(timings).map(|(state, _)| state)
    }

    fn update(&self, path: &str, timings: &mut Timings) -> Result<(), Error> {
        let open_started = Instant::now();
        let _lock = self.lock(true)?;
        timings.open += open_started.elapsed();
        let (mut state, valid_journal_bytes) = self.load_unlocked(timings)?;
        state.update(path)?;
        let record = state
            .records
            .iter()
            .find(|record| record.path == path)
            .expect("updated record exists");
        let frame = encode_journal_frame(state.tick, record)?;

        let commit_started = Instant::now();
        let mut journal = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(self.journal_path())?;
        if journal.metadata()?.len() != valid_journal_bytes as u64 {
            journal.set_len(valid_journal_bytes as u64)?;
        }
        if std::env::var_os("ZFZ_FAULT").as_deref() == Some("journal-partial".as_ref()) {
            journal.write_all(&frame[..frame.len() / 2])?;
            journal.sync_all()?;
            std::process::abort();
        }
        journal.write_all(&frame)?;
        fault("journal-written");
        journal.sync_all()?;
        fault("journal-synced");
        timings.commit += commit_started.elapsed();
        Ok(())
    }

    fn verify(&self) -> Result<State, Error> {
        self.load(&mut Timings::default())
    }

    fn bytes(&self) -> Result<u64, Error> {
        directory_bytes(&self.root)
    }
}

fn fault(point: &str) {
    if std::env::var("ZFZ_FAULT").as_deref() == Ok(point) {
        std::process::abort();
    }
}

fn encode_snapshot(state: &State) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(SNAPSHOT_MAGIC);
    put_u64(&mut bytes, state.tick);
    put_u64(&mut bytes, state.records.len() as u64);
    for record in &state.records {
        encode_record(&mut bytes, record)?;
    }
    let checksum = hash(&bytes);
    put_u32(&mut bytes, checksum);
    Ok(bytes)
}

fn decode_snapshot(bytes: &[u8]) -> Result<State, Error> {
    if bytes.len() < SNAPSHOT_MAGIC.len() + 20 || &bytes[..8] != SNAPSHOT_MAGIC {
        return Err(Error::Invalid("invalid snapshot header".into()));
    }
    let data_len = bytes.len() - 4;
    let expected = u32::from_le_bytes(bytes[data_len..].try_into().unwrap());
    if hash(&bytes[..data_len]) != expected {
        return Err(Error::Invalid("snapshot checksum mismatch".into()));
    }
    let mut cursor = Cursor::new(&bytes[8..data_len]);
    let tick = cursor.u64()?;
    let count = cursor.u64()?;
    let count =
        usize::try_from(count).map_err(|_| Error::Invalid("record count overflow".into()))?;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(cursor.record()?);
    }
    if !cursor.remaining().is_empty() {
        return Err(Error::Invalid("trailing snapshot data".into()));
    }
    Ok(State { tick, records })
}

fn encode_journal_frame(tick: u64, record: &DirectoryRecord) -> Result<Vec<u8>, Error> {
    let mut payload = Vec::new();
    put_u64(&mut payload, tick);
    encode_record(&mut payload, record)?;
    let mut frame = Vec::with_capacity(payload.len() + 8);
    put_u32(
        &mut frame,
        u32::try_from(payload.len())
            .map_err(|_| Error::Invalid("journal frame too large".into()))?,
    );
    frame.extend_from_slice(&payload);
    put_u32(&mut frame, hash(&payload));
    Ok(frame)
}

fn replay_journal(state: &mut State, bytes: &[u8]) -> Result<usize, Error> {
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes.len() - offset < 4 {
            break;
        }
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if length > MAX_PATH_BYTES + 32 {
            return Err(Error::Invalid("unreasonable journal frame length".into()));
        }
        let frame_end = offset + 4 + length + 4;
        if frame_end > bytes.len() {
            break;
        }
        let payload = &bytes[offset + 4..offset + 4 + length];
        let expected =
            u32::from_le_bytes(bytes[offset + 4 + length..frame_end].try_into().unwrap());
        if hash(payload) != expected {
            return Err(Error::Invalid("journal checksum mismatch".into()));
        }
        let mut cursor = Cursor::new(payload);
        let tick = cursor.u64()?;
        let record = cursor.record()?;
        if !cursor.remaining().is_empty() || record.history.last_tick != tick {
            return Err(Error::Invalid("invalid journal sequence".into()));
        }
        // A crash after atomic snapshot replacement but before journal reset
        // leaves already-compacted frames behind. They are safe to skip.
        if tick > state.tick {
            state.tick = tick;
            match state
                .records
                .binary_search_by(|old| old.path.cmp(&record.path))
            {
                Ok(index) => state.records[index] = record,
                Err(index) => state.records.insert(index, record),
            }
        }
        offset = frame_end;
    }
    Ok(offset)
}

fn encode_record(bytes: &mut Vec<u8>, record: &DirectoryRecord) -> Result<(), Error> {
    let path = record.path.as_bytes();
    if path.len() > MAX_PATH_BYTES {
        return Err(Error::Invalid("path too long".into()));
    }
    put_u32(bytes, path.len() as u32);
    bytes.extend_from_slice(path);
    put_u64(bytes, record.history.visits);
    put_u64(bytes, record.history.last_tick);
    put_u64(bytes, record.history.score.to_bits());
    Ok(())
}

fn write_snapshot_bytes_atomic(
    root: &Path,
    snapshot: &[u8],
) -> Result<(std::time::Duration, std::time::Duration), Error> {
    let temporary = root.join("snapshot.new");
    let write_started = Instant::now();
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(snapshot)?;
    file.sync_all()?;
    let write_duration = write_started.elapsed();
    fault("snapshot-temporary-synced");
    let replace_started = Instant::now();
    fs::rename(temporary, root.join("snapshot"))?;
    sync_directory(root)?;
    Ok((write_duration, replace_started.elapsed()))
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.offset..]
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| Error::Invalid("offset overflow".into()))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| Error::Invalid("truncated encoded value".into()))?;
        self.offset = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn record(&mut self) -> Result<DirectoryRecord, Error> {
        let length = self.u32()? as usize;
        if length > MAX_PATH_BYTES {
            return Err(Error::Invalid("unreasonable path length".into()));
        }
        let path = String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| Error::Invalid("path is not UTF-8".into()))?;
        let visits = self.u64()?;
        let last_tick = self.u64()?;
        let score = f64::from_bits(self.u64()?);
        Ok(DirectoryRecord {
            path,
            history: zfz::frecency::Record {
                visits,
                last_tick,
                score,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{JournalStore, decode_snapshot, encode_snapshot};
    use crate::{State, Store, Timings, dataset::generate};

    fn temporary(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("zfz-{name}-{}", std::process::id()))
    }

    #[test]
    fn snapshot_round_trips_unusual_paths() {
        let state = generate(200);
        assert_eq!(
            decode_snapshot(&encode_snapshot(&state).unwrap()).unwrap(),
            state
        );
    }

    #[test]
    fn snapshot_checksum_detects_corruption() {
        let state = generate(20);
        let mut bytes = encode_snapshot(&state).unwrap();
        bytes[16] ^= 1;
        assert!(decode_snapshot(&bytes).is_err());
    }

    #[test]
    fn updates_replay_and_compaction_preserves_state() {
        let root = temporary("journal-compact");
        let _ = fs::remove_dir_all(&root);
        let store = JournalStore::new(&root);
        store.initialise(&generate(100)).unwrap();
        store
            .update("/new path/naïve", &mut Timings::default())
            .unwrap();
        let before = store.verify().unwrap();
        store.compact(&mut Timings::default()).unwrap();
        assert_eq!(store.verify().unwrap(), before);
        assert_eq!(fs::metadata(root.join("journal")).unwrap().len(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_truncated_final_frame_is_ignored() {
        let root = temporary("journal-truncated");
        let _ = fs::remove_dir_all(&root);
        let store = JournalStore::new(&root);
        let initial = State::empty();
        store.initialise(&initial).unwrap();
        store.update("/complete", &mut Timings::default()).unwrap();
        let mut journal = fs::read(root.join("journal")).unwrap();
        journal.extend_from_slice(&[20, 0, 0, 0, 1, 2, 3]);
        fs::write(root.join("journal"), journal).unwrap();
        assert_eq!(store.verify().unwrap().records[0].path, "/complete");
        fs::remove_dir_all(root).unwrap();
    }
}
