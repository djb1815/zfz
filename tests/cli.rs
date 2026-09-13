use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rstest::rstest;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("zfz-cli-{name}-{unique}-{sequence}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn zfz(root: &TestDirectory, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_zfz"))
        .args(arguments)
        .env("XDG_STATE_HOME", root.path().join("state"))
        .output()
        .unwrap()
}

fn successful(root: &TestDirectory, arguments: &[&str]) -> Output {
    let output = zfz(root, arguments);
    assert!(
        output.status.success(),
        "zfz {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn path_text(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn help_does_not_require_a_state_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_zfz"))
        .arg("--help")
        .env_remove("XDG_STATE_HOME")
        .env_remove("HOME")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout).unwrap().contains("Usage:"));
}

#[test]
fn executable_adds_queries_lists_and_removes_records() {
    let root = TestDirectory::new("operations");
    let frequent = root.path().join("frequent-docs");
    let recent = root.path().join("recent-docs");
    fs::create_dir_all(&frequent).unwrap();
    fs::create_dir_all(&recent).unwrap();

    successful(&root, &["--add", path_text(&frequent)]);
    successful(&root, &["-a", path_text(&frequent)]);
    successful(&root, &["--add", path_text(&recent)]);

    let ranked = successful(&root, &["--rank", "docs"]);
    assert_eq!(
        ranked.stdout,
        format!("{}\n", frequent.display()).as_bytes()
    );

    let timed = successful(&root, &["-t", "docs"]);
    assert_eq!(timed.stdout, format!("{}\n", recent.display()).as_bytes());

    let listed = successful(&root, &["--list", "--null", "docs"]);
    let records = listed.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    assert_eq!(
        records,
        [
            frequent.as_os_str().as_encoded_bytes(),
            recent.as_os_str().as_encoded_bytes(),
            b""
        ]
    );

    successful(&root, &["--remove", path_text(&frequent)]);
    let listed = successful(&root, &["-l", "docs"]);
    assert_eq!(listed.stdout, format!("{}\n", recent.display()).as_bytes());
}

#[test]
fn current_restriction_uses_path_component_boundaries() {
    let root = TestDirectory::new("current");
    let current = root.path().join("work");
    let inside = current.join("inside-docs");
    let lookalike = root.path().join("workspace/outside-docs");
    fs::create_dir_all(&inside).unwrap();
    fs::create_dir_all(&lookalike).unwrap();
    successful(&root, &["-a", path_text(&inside)]);
    successful(&root, &["-a", path_text(&lookalike)]);

    let output = Command::new(env!("CARGO_BIN_EXE_zfz"))
        .args(["--current", "docs"])
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env("PWD", &current)
        .current_dir(&current)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, format!("{}\n", inside.display()).as_bytes());
}

#[test]
fn selection_removes_stale_candidates_and_tries_the_next_match() {
    let root = TestDirectory::new("stale");
    let live = root.path().join("live-stale-project");
    let stale = root.path().join("missing-stale-project");
    fs::create_dir_all(&live).unwrap();
    successful(&root, &["-a", path_text(&live)]);
    successful(&root, &["-a", path_text(&stale)]);

    let selected = successful(&root, &["stale"]);
    assert_eq!(selected.stdout, format!("{}\n", live.display()).as_bytes());
    let listed = successful(&root, &["-l", "stale"]);
    assert_eq!(listed.stdout, format!("{}\n", live.display()).as_bytes());
}

#[test]
fn null_output_preserves_a_path_containing_a_newline() {
    let root = TestDirectory::new("newline");
    let unusual = root.path().join("line\nbreak-docs");
    fs::create_dir_all(&unusual).unwrap();
    successful(&root, &["-a", path_text(&unusual)]);

    let output = successful(&root, &["-e0", "docs"]);
    let mut expected = unusual.as_os_str().as_encoded_bytes().to_vec();
    expected.push(0);
    assert_eq!(output.stdout, expected);
}

#[test]
fn jump_emits_exactly_one_null_terminated_unusual_path() {
    let root = TestDirectory::new("jump-newline");
    let unusual = root.path().join("line\nbreak-[docs]");
    fs::create_dir_all(&unusual).unwrap();
    successful(&root, &["--add", path_text(&unusual)]);

    let output = successful(&root, &["--jump", "docs"]);
    let mut expected = unusual.as_os_str().as_encoded_bytes().to_vec();
    expected.push(0);
    assert_eq!(output.stdout, expected);
    assert_eq!(output.stdout.iter().filter(|byte| **byte == 0).count(), 1);
}

#[rstest]
#[case::echo(&["--echo", "docs"])]
#[case::echo_short(&["-e", "docs"])]
#[case::list(&["--list", "docs"])]
#[case::list_short(&["-l", "docs"])]
#[case::null(&["--null", "docs"])]
#[case::null_short(&["-0", "docs"])]
#[case::help(&["--help"])]
#[case::add(&["--add", "{new}"])]
#[case::track(&["--track", "{new}"])]
#[case::remove(&["--remove", "{existing}"])]
#[case::remove_recursive(&["--remove-recursive", "{existing}"])]
fn jump_rejects_output_and_administrative_options_without_mutation(
    #[case] trailing_arguments: &[&str],
) {
    let root = TestDirectory::new("jump-rejection");
    let existing = root.path().join("existing-docs");
    fs::create_dir_all(&existing).unwrap();
    successful(&root, &["--add", path_text(&existing)]);
    let before = successful(&root, &["--list"]).stdout;

    let new = root.path().join("new-path");
    let resolved = trailing_arguments
        .iter()
        .map(|argument| match *argument {
            "{existing}" => path_text(&existing),
            "{new}" => path_text(&new),
            other => other,
        })
        .collect::<Vec<_>>();
    let mut arguments = vec!["--jump"];
    arguments.extend(resolved);
    let rejected = zfz(&root, &arguments);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    assert_eq!(successful(&root, &["--list"]).stdout, before);
}

#[test]
fn no_match_and_unsafe_root_removal_have_distinct_failures() {
    let root = TestDirectory::new("failures");
    let no_match = zfz(&root, &["missing"]);
    assert_eq!(no_match.status.code(), Some(1));
    assert!(no_match.stdout.is_empty());

    successful(&root, &["-a", "/record"]);
    let refused = zfz(&root, &["-X", "/"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--force"));
    let still_present = successful(&root, &["-l"]);
    assert_eq!(still_present.stdout, b"/record\n");
    successful(&root, &["-X", "/", "--force"]);
    assert!(successful(&root, &["-l"]).stdout.is_empty());
}

#[rstest]
#[case(None, "z")]
#[case(Some("j"), "j")]
#[case(Some("zfz"), "zfz")]
fn fish_wrapper_consumes_one_null_terminated_unusual_path(
    #[case] configured_command: Option<&str>,
    #[case] invoked_command: &str,
) {
    let root = TestDirectory::new("fish");
    let selected = root.path().join("line\nbreak docs");
    fs::create_dir_all(&selected).unwrap();
    successful(&root, &["-a", path_text(&selected)]);

    let binary_directory = Path::new(env!("CARGO_BIN_EXE_zfz")).parent().unwrap();
    let mut search_paths = vec![binary_directory.to_path_buf()];
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut command = Command::new("fish");
    command
        .args([
            "--no-config",
            "-c",
            "set --prepend fish_function_path shell/fish/functions; source shell/fish/conf.d/zfz.fish; functions --query \"$argv[2]\"; or exit 2; \"$argv[2]\" docs; and test \"$PWD\" = \"$argv[1]\"",
            path_text(&selected),
            invoked_command,
        ])
        .env("PATH", env::join_paths(search_paths).unwrap())
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env_remove("ZFZ_CMD")
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if let Some(configured_command) = configured_command {
        command.env("ZFZ_CMD", configured_command);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "fish wrapper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn empty_fish_command_configuration_creates_no_alias() {
    let root = TestDirectory::new("fish-empty-command");
    let output = fish(
        &root,
        "source shell/fish/conf.d/zfz.fish; not functions --query z; and not functions --query zfz; and command zfz --list",
        &[],
        Some(""),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fish_alias_without_arguments_reserves_interactive_selection() {
    let root = TestDirectory::new("fish-no-arguments");
    let output = fish(
        &root,
        "set --prepend fish_function_path shell/fish/functions; source shell/fish/conf.d/zfz.fish; z",
        &[],
        None,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("interactive selection is not implemented yet")
    );
}

#[rstest]
#[case("-h")]
#[case("--help")]
fn fish_alias_help_is_forwarded_without_navigation(#[case] option: &str) {
    let root = TestDirectory::new("fish-help");
    let output = fish(
        &root,
        "set --prepend fish_function_path shell/fish/functions; source shell/fish/conf.d/zfz.fish; set before $PWD; z $argv[1]; and test \"$PWD\" = \"$before\"",
        &[option],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("--jump"));
}

#[rstest]
#[case(&["-r", "-c", "docs"])]
#[case(&["--rank", "--current", "docs"])]
#[case(&["-t", "docs"])]
#[case(&["--time", "docs"])]
fn fish_alias_accepts_ranking_and_current_modifiers(#[case] arguments: &[&str]) {
    let root = TestDirectory::new("fish-modifiers");
    let selected = root.path().join("selected-docs");
    fs::create_dir_all(&selected).unwrap();
    successful(&root, &["--add", path_text(&selected)]);

    let output = fish(
        &root,
        "set --prepend fish_function_path shell/fish/functions; source shell/fish/conf.d/zfz.fish; builtin cd -- \"$argv[1]\"; or exit 3; z $argv[3..]; and test \"$PWD\" = \"$argv[2]\"",
        &std::iter::once(path_text(root.path()))
            .chain(std::iter::once(path_text(&selected)))
            .chain(arguments.iter().copied())
            .collect::<Vec<_>>(),
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn executable_operations_remain_direct_and_alias_operations_are_rejected() {
    let root = TestDirectory::new("fish-interface-split");
    let selected = root.path().join("selected-docs");
    fs::create_dir_all(&selected).unwrap();
    successful(&root, &["--add", path_text(&selected)]);

    let output = fish(
        &root,
        "set --prepend fish_function_path shell/fish/functions; source shell/fish/conf.d/zfz.fish; command zfz --list docs; or exit 3; command zfz --remove \"$argv[1]\"; or exit 4; command zfz --add \"$argv[1]\"; or exit 5; z --list docs >/dev/null 2>/dev/null; test $status -eq 2; or exit 6; z --echo docs >/dev/null 2>/dev/null; test $status -eq 2; or exit 7; z --remove \"$argv[1]\" >/dev/null 2>/dev/null; test $status -eq 2; or exit 8; command zfz --list docs",
        &[path_text(&selected)],
        None,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!("{0}\n{0}\n", selected.display());
    assert_eq!(output.stdout, expected.as_bytes());
}

fn fish(
    root: &TestDirectory,
    script: &str,
    arguments: &[&str],
    configured_command: Option<&str>,
) -> Output {
    let binary_directory = Path::new(env!("CARGO_BIN_EXE_zfz")).parent().unwrap();
    let function_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("shell/fish/functions");
    let mut search_paths = vec![binary_directory.to_path_buf()];
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let mut command = Command::new("fish");
    command
        .args([
            "--no-config",
            "-c",
            &format!(
                "set --prepend fish_function_path '{}'; {script}",
                path_text(&function_directory)
            ),
            "--",
        ])
        .args(arguments)
        .env("PATH", env::join_paths(search_paths).unwrap())
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env_remove("ZFZ_CMD")
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if let Some(configured_command) = configured_command {
        command.env("ZFZ_CMD", configured_command);
    }
    command.output().unwrap()
}

#[test]
fn fish_pwd_handler_tracks_directory_changes() {
    let root = TestDirectory::new("tracking");
    let tracked = root.path().join("event-tracked-project");
    fs::create_dir_all(&tracked).unwrap();

    let binary_directory = Path::new(env!("CARGO_BIN_EXE_zfz")).parent().unwrap();
    let mut search_paths = vec![binary_directory.to_path_buf()];
    search_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let output = Command::new("fish")
        .args([
            "--no-config",
            "-c",
            "source shell/fish/conf.d/zfz.fish; cd \"$argv[1]\"",
            path_text(&tracked),
        ])
        .env("PATH", env::join_paths(search_paths).unwrap())
        .env("XDG_STATE_HOME", root.path().join("state"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Fish tracking failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let listed = successful(&root, &["--list", "tracked"]);
    assert_eq!(listed.stdout, format!("{}\n", tracked.display()).as_bytes());
}
