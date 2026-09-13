//! Command-line orchestration for matching, ranking, and persistent history.

use std::{
    env,
    error::Error,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::{
    matcher::match_terms,
    ranking::{Candidate, HistoryMode, RankingError, rank},
    storage::{Database, StorageError, VisitOutcome},
};
use lexopt::prelude::{Long, Short, Value, ValueExt};

const HELP: &str = "zfz - a fast directory-history query tool

Usage:
  zfz [OPTIONS] QUERY...
  zfz -l [OPTIONS] [QUERY...]
  zfz -a PATH
  zfz -x PATH
  zfz -X PATH [--force]

Options:
  -a, --add PATH               Add or update a directory
  -c, --current                Restrict results to the current directory tree
  -e, --echo                   Print the best matching directory
  -l, --list                   Print every match in ranked order
  -r, --rank                   Rank by total visit count
  -t, --time                   Rank by most recent visit
  -x, --remove PATH            Remove an exact directory
  -X, --remove-recursive PATH  Remove a directory tree from history
  -0, --null                   End output records with NUL instead of newline
      --force                  Allow --remove-recursive / to clear all history
  -h, --help                   Print help

Use -- to pass query terms beginning with a hyphen. No match exits with status 1.
";

/// Errors produced while interpreting or executing a CLI request.
#[derive(Debug)]
pub enum CliError {
    /// The arguments do not describe a valid operation.
    Usage(String),
    /// No usable directory matched the query.
    NoMatch,
    /// The current directory cannot be represented by the history model.
    CurrentDirectory(String),
    /// Persistent history failed.
    Storage(StorageError),
    /// Canonical ranking failed.
    Ranking(RankingError),
    /// Writing command output failed.
    Output(io::Error),
}

impl CliError {
    /// Returns the process exit status associated with this failure.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        if matches!(self, Self::NoMatch) { 1 } else { 2 }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::CurrentDirectory(message) => message.fmt(formatter),
            Self::NoMatch => formatter.write_str("no matching directory"),
            Self::Storage(error) => error.fmt(formatter),
            Self::Ranking(error) => error.fmt(formatter),
            Self::Output(error) => error.fmt(formatter),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Ranking(error) => Some(error),
            Self::Output(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StorageError> for CliError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<RankingError> for CliError {
    fn from(value: RankingError) -> Self {
        Self::Ranking(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    Query,
    Add(String),
    Track(String),
    Remove(String),
    RemoveRecursive(String),
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Arguments {
    operation: Operation,
    terms: Vec<String>,
    list: bool,
    echo: bool,
    current: bool,
    null: bool,
    force: bool,
    history_mode: HistoryMode,
}

impl Default for Arguments {
    fn default() -> Self {
        Self {
            operation: Operation::Query,
            terms: Vec::new(),
            list: false,
            echo: false,
            current: false,
            null: false,
            force: false,
            history_mode: HistoryMode::Frecency,
        }
    }
}

/// Runs the process arguments against the default history database.
pub fn run_from_env() -> Result<(), CliError> {
    let arguments = parse(env::args_os().skip(1))?;
    let database = Database::new(Database::default_path()?);
    let output = execute(arguments, &database)?;
    io::stdout()
        .lock()
        .write_all(&output)
        .map_err(CliError::Output)
}

fn parse(
    arguments: impl IntoIterator<Item = impl Into<std::ffi::OsString>>,
) -> Result<Arguments, CliError> {
    let mut parsed = Arguments::default();
    let mut parser = lexopt::Parser::from_args(arguments);
    let mut selected_mode = None;

    while let Some(argument) = parser
        .next()
        .map_err(|error| CliError::Usage(error.to_string()))?
    {
        match argument {
            Short('h') | Long("help") => parsed.operation = Operation::Help,
            Short('e') | Long("echo") => parsed.echo = true,
            Short('l') | Long("list") => parsed.list = true,
            Short('r') | Long("rank") => {
                set_history_mode(&mut parsed, &mut selected_mode, HistoryMode::Frequency)?
            }
            Short('t') | Long("time") => {
                set_history_mode(&mut parsed, &mut selected_mode, HistoryMode::Recency)?
            }
            Short('c') | Long("current") => parsed.current = true,
            Short('0') | Long("null") => parsed.null = true,
            Long("force") => parsed.force = true,
            Short('a') | Long("add") => {
                let path = option_value(&mut parser, "--add")?;
                set_operation(&mut parsed, Operation::Add(path))?;
            }
            Long("track") => {
                let path = option_value(&mut parser, "--track")?;
                set_operation(&mut parsed, Operation::Track(path))?;
            }
            Short('x') | Long("remove") => {
                let path = option_value(&mut parser, "--remove")?;
                set_operation(&mut parsed, Operation::Remove(path))?;
            }
            Short('X') | Long("remove-recursive") => {
                let path = option_value(&mut parser, "--remove-recursive")?;
                set_operation(&mut parsed, Operation::RemoveRecursive(path))?;
            }
            Value(term) => parsed.terms.push(
                term.string()
                    .map_err(|error| CliError::Usage(error.to_string()))?,
            ),
            _ => return Err(CliError::Usage(argument.unexpected().to_string())),
        }
    }

    validate(parsed)
}

fn option_value(parser: &mut lexopt::Parser, option: &str) -> Result<String, CliError> {
    parser
        .value()
        .map_err(|error| CliError::Usage(format!("{option}: {error}")))?
        .string()
        .map_err(|error| CliError::Usage(format!("{option} path: {error}")))
}

fn set_history_mode(
    parsed: &mut Arguments,
    selected: &mut Option<HistoryMode>,
    mode: HistoryMode,
) -> Result<(), CliError> {
    if selected.is_some_and(|existing| existing != mode) {
        return Err(CliError::Usage(
            "--rank and --time cannot be used together".into(),
        ));
    }
    *selected = Some(mode);
    parsed.history_mode = mode;
    Ok(())
}

fn set_operation(parsed: &mut Arguments, operation: Operation) -> Result<(), CliError> {
    if matches!(parsed.operation, Operation::Help) {
        return Ok(());
    }
    if !matches!(parsed.operation, Operation::Query) {
        return Err(CliError::Usage(
            "only one administrative operation may be used".into(),
        ));
    }
    parsed.operation = operation;
    Ok(())
}

fn validate(parsed: Arguments) -> Result<Arguments, CliError> {
    if matches!(parsed.operation, Operation::Help) {
        return Ok(parsed);
    }
    if parsed.echo && parsed.list {
        return Err(CliError::Usage(
            "--echo and --list cannot be used together".into(),
        ));
    }
    match &parsed.operation {
        Operation::Query => {
            if parsed.force {
                return Err(CliError::Usage(
                    "--force requires --remove-recursive /".into(),
                ));
            }
            if parsed.terms.is_empty() && !parsed.list {
                return Err(CliError::Usage(
                    "a query is required unless --list is used".into(),
                ));
            }
        }
        Operation::Add(_) | Operation::Track(_) | Operation::Remove(_) => {
            validate_administrative_modifiers(&parsed)?;
            if parsed.force {
                return Err(CliError::Usage(
                    "--force requires --remove-recursive /".into(),
                ));
            }
        }
        Operation::RemoveRecursive(path) => {
            validate_administrative_modifiers(&parsed)?;
            let root = recursive_root(path);
            if root == "/" && !parsed.force {
                return Err(CliError::Usage(
                    "refusing to clear all history; repeat with --force to confirm".into(),
                ));
            }
            if root != "/" && parsed.force {
                return Err(CliError::Usage(
                    "--force is only valid with --remove-recursive /".into(),
                ));
            }
        }
        Operation::Help => unreachable!(),
    }
    Ok(parsed)
}

fn validate_administrative_modifiers(parsed: &Arguments) -> Result<(), CliError> {
    if !parsed.terms.is_empty()
        || parsed.list
        || parsed.echo
        || parsed.current
        || parsed.null
        || parsed.history_mode != HistoryMode::Frecency
    {
        return Err(CliError::Usage(
            "administrative operations cannot be combined with query or output options".into(),
        ));
    }
    Ok(())
}

fn execute(arguments: Arguments, database: &Database) -> Result<Vec<u8>, CliError> {
    match arguments.operation {
        Operation::Help => Ok(HELP.as_bytes().to_vec()),
        Operation::Add(path) => {
            database.add(&path)?;
            Ok(Vec::new())
        }
        Operation::Track(path) => {
            let _outcome: VisitOutcome = database.record_visit(&path)?;
            Ok(Vec::new())
        }
        Operation::Remove(path) => {
            database.remove_exact(&path)?;
            Ok(Vec::new())
        }
        Operation::RemoveRecursive(path) => {
            database.remove_recursive(&path)?;
            Ok(Vec::new())
        }
        Operation::Query => execute_query(arguments, database),
    }
}

fn execute_query(arguments: Arguments, database: &Database) -> Result<Vec<u8>, CliError> {
    let history = database.load_candidates()?;
    let current_directory = arguments.current.then(current_directory).transpose()?;
    let mut candidates = history
        .records()
        .iter()
        .filter(|record| {
            current_directory
                .as_ref()
                .is_none_or(|current| Path::new(record.path()).starts_with(current))
        })
        .filter_map(|record| {
            match_terms(record.path(), arguments.terms.iter().map(String::as_str)).map(|matched| {
                Candidate::new(record.path(), record.history(), matched.fuzzy_score())
            })
        })
        .collect::<Vec<_>>();
    rank(&mut candidates, arguments.history_mode, history.tick())?;

    let separator = if arguments.null { b'\0' } else { b'\n' };
    if arguments.list {
        let mut output = Vec::new();
        for candidate in candidates {
            output.extend_from_slice(candidate.path().as_bytes());
            output.push(separator);
        }
        return Ok(output);
    }

    for candidate in candidates {
        if fs::metadata(candidate.path()).is_ok_and(|metadata| metadata.is_dir()) {
            let mut output = candidate.path().as_bytes().to_vec();
            output.push(separator);
            return Ok(output);
        }
        database.remove_exact(candidate.path())?;
    }
    Err(CliError::NoMatch)
}

fn current_directory() -> Result<PathBuf, CliError> {
    if let Some(pwd) = env::var_os("PWD") {
        let pwd = pwd.into_string().map_err(|_| {
            CliError::CurrentDirectory("PWD must be valid UTF-8 for --current".into())
        })?;
        let path = PathBuf::from(pwd);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    env::current_dir().map_err(|error| {
        CliError::CurrentDirectory(format!("cannot determine current directory: {error}"))
    })
}

fn recursive_root(path: &str) -> &str {
    let root = path.trim_end_matches('/');
    if root.is_empty() { "/" } else { root }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{CliError, HistoryMode, Operation, parse};

    fn arguments(values: &[&str]) -> Result<super::Arguments, CliError> {
        parse(values.iter().map(OsString::from))
    }

    #[test]
    fn parses_query_modes_and_short_option_clusters() {
        let parsed = arguments(&["-lc0r", "docs", "proj"]).unwrap();
        assert_eq!(parsed.operation, Operation::Query);
        assert!(parsed.list);
        assert!(parsed.current);
        assert!(parsed.null);
        assert_eq!(parsed.history_mode, HistoryMode::Frequency);
        assert_eq!(parsed.terms, ["docs", "proj"]);
    }

    #[test]
    fn accepts_attached_administrative_paths() {
        assert_eq!(
            arguments(&["--add=/path with spaces"]).unwrap().operation,
            Operation::Add("/path with spaces".into())
        );
        assert_eq!(
            arguments(&["-x/path"]).unwrap().operation,
            Operation::Remove("/path".into())
        );
        assert_eq!(
            arguments(&["-a=/path"]).unwrap().operation,
            Operation::Add("/path".into())
        );
    }

    #[test]
    fn rejects_incompatible_modes_and_operations() {
        for values in [
            &["-r", "-t", "query"][..],
            &["-e", "-l", "query"],
            &["--add", "/path", "query"],
            &["--remove", "/path", "--null"],
        ] {
            assert!(matches!(arguments(values), Err(CliError::Usage(_))));
        }
    }

    #[test]
    fn rejects_unknown_options_missing_values_and_values_on_switches() {
        for values in [&["--unknown"][..], &["--add"], &["--echo=value", "query"]] {
            assert!(matches!(arguments(values), Err(CliError::Usage(_))));
        }
    }

    #[test]
    fn root_recursive_removal_requires_explicit_force() {
        assert!(matches!(arguments(&["-X", "/"]), Err(CliError::Usage(_))));
        assert!(arguments(&["-X", "////", "--force"]).is_ok());
        assert!(matches!(
            arguments(&["-X", "/work", "--force"]),
            Err(CliError::Usage(_))
        ));
    }

    #[test]
    fn double_dash_allows_hyphenated_query_terms() {
        assert_eq!(arguments(&["--", "-docs"]).unwrap().terms, ["-docs"]);
    }

    #[test]
    fn help_cannot_be_replaced_by_a_later_destructive_operation() {
        assert_eq!(
            arguments(&["--help", "--remove-recursive", "/", "--force"])
                .unwrap()
                .operation,
            Operation::Help
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_query_terms() {
        use std::os::unix::ffi::OsStringExt;

        assert!(matches!(
            parse([OsString::from_vec(vec![0xff])]),
            Err(CliError::Usage(_))
        ));
    }
}
