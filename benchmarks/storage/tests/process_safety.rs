use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Barrier},
    thread,
};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_zfz-storage-benchmark")
}

const BACKENDS: [&str; 5] = [
    "journal",
    "sqlite-delete",
    "sqlite-delete-without-rowid",
    "sqlite-delete-production",
    "sqlite-wal",
];

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("zfz-process-{name}-{}", std::process::id()))
}

fn run(arguments: &[&str]) {
    let output = Command::new(binary()).args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify(backend: &str, root: &Path) -> (u64, usize) {
    let output = Command::new(binary())
        .args(["verify", backend, root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let mut fields = text.trim().split('\t');
    (
        fields.next().unwrap().parse().unwrap(),
        fields.next().unwrap().parse().unwrap(),
    )
}

#[test]
fn concurrent_process_writers_do_not_lose_updates() {
    for backend in BACKENDS {
        let root = temporary(backend);
        let _ = fs::remove_dir_all(&root);
        run(&["init", backend, root.to_str().unwrap(), "100"]);
        let initial_tick = verify(backend, &root).0;
        let workers: Vec<_> = (0..16)
            .map(|index| {
                let root = root.clone();
                thread::spawn(move || {
                    let path = format!("/concurrent/{index}");
                    run(&["update", backend, root.to_str().unwrap(), &path]);
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let (tick, records) = verify(backend, &root);
        assert_eq!(tick, initial_tick + 16, "{backend}");
        assert_eq!(records, 116, "{backend}");
        fs::remove_dir_all(root).unwrap();
    }
}

fn reader_writer_overlap_round(backend: &'static str, round: usize) {
    let root = temporary(&format!("overlap-{backend}-{round}"));
    let _ = fs::remove_dir_all(&root);
    run(&["init", backend, root.to_str().unwrap(), "100"]);
    let initial = verify(backend, &root);
    let barrier = Arc::new(Barrier::new(16));
    let workers: Vec<_> = (0..16)
        .map(|index| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                if index < 8 {
                    run(&["query", backend, root.to_str().unwrap(), "projects"]);
                } else {
                    let path = format!("/overlap/{round}/{index}");
                    run(&["update", backend, root.to_str().unwrap(), &path]);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let recovered = verify(backend, &root);
    assert_eq!(recovered.0, initial.0 + 8, "{backend}, round {round}");
    assert_eq!(recovered.1, initial.1 + 8, "{backend}, round {round}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_readers_and_reader_writer_overlap_see_valid_state() {
    for backend in BACKENDS {
        reader_writer_overlap_round(backend, 0);
    }
}

#[test]
#[ignore = "explicit storage stress experiment"]
fn reader_writer_overlap_survives_one_hundred_rounds() {
    for round in 1..=100 {
        for backend in BACKENDS {
            reader_writer_overlap_round(backend, round);
        }
    }
}

#[test]
fn interrupted_updates_recover_and_accept_the_next_update() {
    let cases = [
        ("journal", "journal-partial", Some(false)),
        // A complete frame killed before fsync may either survive or disappear;
        // both outcomes are valid so long as it never becomes corrupt.
        ("journal", "journal-written", None),
        ("journal", "journal-synced", Some(true)),
        ("sqlite-delete", "sqlite-before-commit", Some(false)),
        ("sqlite-delete", "sqlite-after-commit", Some(true)),
        (
            "sqlite-delete-without-rowid",
            "sqlite-before-commit",
            Some(false),
        ),
        (
            "sqlite-delete-without-rowid",
            "sqlite-after-commit",
            Some(true),
        ),
        (
            "sqlite-delete-production",
            "sqlite-before-commit",
            Some(false),
        ),
        (
            "sqlite-delete-production",
            "sqlite-after-commit",
            Some(true),
        ),
        ("sqlite-wal", "sqlite-before-commit", Some(false)),
        ("sqlite-wal", "sqlite-after-commit", Some(true)),
    ];
    for (backend, fault, committed) in cases {
        let root = temporary(&format!("{backend}-{fault}"));
        let _ = fs::remove_dir_all(&root);
        run(&["init", backend, root.to_str().unwrap(), "100"]);
        let initial = verify(backend, &root);
        let status = Command::new(binary())
            .env("ZFZ_FAULT", fault)
            .args(["update", backend, root.to_str().unwrap(), "/crashed"])
            .status()
            .unwrap();
        assert!(!status.success());
        let recovered = verify(backend, &root);
        if let Some(committed) = committed {
            assert_eq!(recovered.0, initial.0 + u64::from(committed), "{fault}");
        } else {
            assert!(recovered.0 == initial.0 || recovered.0 == initial.0 + 1);
        }
        run(&["update", backend, root.to_str().unwrap(), "/after-crash"]);
        assert_eq!(verify(backend, &root).0, recovered.0 + 1, "{fault}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn interrupted_compaction_never_exposes_partial_state() {
    for fault in ["snapshot-temporary-synced", "compact-snapshot-replaced"] {
        let root = temporary(fault);
        let _ = fs::remove_dir_all(&root);
        run(&["init", "journal", root.to_str().unwrap(), "100"]);
        run(&[
            "update",
            "journal",
            root.to_str().unwrap(),
            "/before-compact",
        ]);
        let expected = verify("journal", &root);
        let status = Command::new(binary())
            .env("ZFZ_FAULT", fault)
            .args(["compact", "journal", root.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(!status.success());
        assert_eq!(verify("journal", &root), expected, "{fault}");
        run(&[
            "update",
            "journal",
            root.to_str().unwrap(),
            "/after-compact",
        ]);
        assert_eq!(verify("journal", &root).0, expected.0 + 1);
        fs::remove_dir_all(root).unwrap();
    }
}
