//! Benchmark-only persistence prototypes.

pub mod dataset;
pub mod journal;
#[cfg(feature = "sqlite")]
pub mod sqlite;

use std::{fmt, io, path::Path, time::Duration};

use zfz::{
    frecency::Record,
    matcher::match_terms,
    ranking::{Candidate, HistoryMode, rank},
};

#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryRecord {
    pub path: String,
    pub history: Record,
}

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub tick: u64,
    pub records: Vec<DirectoryRecord>,
}

impl State {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            tick: 0,
            records: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        let mut previous: Option<&str> = None;
        for record in &self.records {
            if record.path.as_bytes().contains(&0) {
                return Err(Error::Invalid("path contains NUL".into()));
            }
            if record.history.last_tick() > self.tick {
                return Err(Error::Invalid(format!(
                    "{} has last tick {} after global tick {}",
                    record.path,
                    record.history.last_tick(),
                    self.tick
                )));
            }
            if !stored_score(record.history).is_finite() {
                return Err(Error::Invalid("record score is not finite".into()));
            }
            if previous.is_some_and(|value| value >= record.path.as_str()) {
                return Err(Error::Invalid(
                    "records are not uniquely sorted by path".into(),
                ));
            }
            previous = Some(&record.path);
        }
        Ok(())
    }

    pub fn update(&mut self, path: &str) -> Result<(), Error> {
        self.tick = self
            .tick
            .checked_add(1)
            .ok_or_else(|| Error::Invalid("event clock overflow".into()))?;
        match self
            .records
            .binary_search_by(|record| record.path.as_str().cmp(path))
        {
            Ok(index) => {
                self.records[index].history = visit(self.records[index].history, self.tick)?;
            }
            Err(index) => self.records.insert(
                index,
                DirectoryRecord {
                    path: path.to_owned(),
                    history: first_visit(self.tick)?,
                },
            ),
        }
        Ok(())
    }
}

/// Builds a validated history record from benchmark persistence data.
pub(crate) fn record_from_parts(visits: u64, last_tick: u64, score: f64) -> Result<Record, Error> {
    Record::new(visits, last_tick, score)
        .map_err(|error| Error::Invalid(format!("invalid record: {error}")))
}

/// Records an initial visit using the production frecency model's public API.
pub(crate) fn first_visit(tick: u64) -> Result<Record, Error> {
    record_from_parts(1, tick, 1.0)
}

/// Applies one visit using the production frecency model's default decay.
pub(crate) fn visit(record: Record, tick: u64) -> Result<Record, Error> {
    if tick <= record.last_tick() {
        return Err(Error::Invalid(
            "each visit must advance the event clock".into(),
        ));
    }
    let visits = record
        .visits()
        .checked_add(1)
        .ok_or_else(|| Error::Invalid("visit count overflow".into()))?;
    record_from_parts(visits, tick, record.score_at(tick) + 1.0)
}

/// Returns the score as persisted at a record's most recent event-clock tick.
#[must_use]
pub(crate) fn stored_score(record: Record) -> f64 {
    record.score_at(record.last_tick())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Timings {
    pub open: Duration,
    pub load: Duration,
    pub decode: Duration,
    pub match_rank: Duration,
    pub encode: Duration,
    pub commit: Duration,
    pub replace: Duration,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    #[cfg(feature = "sqlite")]
    Sql(rusqlite::Error),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            #[cfg(feature = "sqlite")]
            Self::Sql(error) => error.fmt(formatter),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(feature = "sqlite")]
impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

pub trait Store {
    fn initialise(&self, state: &State) -> Result<(), Error>;
    fn load(&self, timings: &mut Timings) -> Result<State, Error>;
    fn update(&self, path: &str, timings: &mut Timings) -> Result<(), Error>;
    fn verify(&self) -> Result<State, Error>;
    fn bytes(&self) -> Result<u64, Error>;
}

pub fn query<'a>(state: &'a State, terms: &[&str]) -> Result<Option<&'a str>, Error> {
    let mut candidates = Vec::new();
    for record in &state.records {
        if let Some(matched) = match_terms(&record.path, terms.iter().copied()) {
            candidates.push(Candidate::new(
                &record.path,
                record.history,
                matched.fuzzy_score(),
            ));
        }
    }
    rank(&mut candidates, HistoryMode::Frecency, state.tick)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    Ok(candidates.first().map(Candidate::path))
}

pub fn directory_bytes(path: &Path) -> Result<u64, Error> {
    let mut total = 0;
    if !path.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::{DirectoryRecord, State, first_visit};

    #[test]
    fn event_clock_overflow_is_rejected_without_mutation() {
        let mut state = State {
            tick: u64::MAX,
            records: vec![DirectoryRecord {
                path: "/existing".into(),
                history: first_visit(1).unwrap(),
            }],
        };
        let original = state.clone();
        assert!(state.update("/new").is_err());
        assert_eq!(state, original);
    }

    #[test]
    fn duplicate_or_unsorted_paths_are_rejected() {
        let state = State {
            tick: 1,
            records: vec![
                DirectoryRecord {
                    path: "/same".into(),
                    history: first_visit(1).unwrap(),
                },
                DirectoryRecord {
                    path: "/same".into(),
                    history: first_visit(1).unwrap(),
                },
            ],
        };
        assert!(state.validate().is_err());
    }
}
