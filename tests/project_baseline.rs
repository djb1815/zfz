use std::path::Path;

#[test]
fn prototype_fixtures_are_present() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    for name in [
        "directory_histories.tsv",
        "matcher_cases.tsv",
        "queries.tsv",
        "ranking_histories.tsv",
    ] {
        let fixture = fixtures.join(name);
        assert!(fixture.is_file(), "missing fixture: {}", fixture.display());
        let contents = std::fs::read_to_string(&fixture).expect("fixture is readable");
        assert!(contents.lines().any(|line| !line.starts_with('#')));
    }
}
