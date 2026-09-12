#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 OUTPUT_DIRECTORY" >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
binary="$script_dir/target/release/zfz-storage-benchmark"
output=$1
work="$output/work"

test -x "$binary"
command -v hyperfine >/dev/null
command -v jq >/dev/null
mkdir -p "$output/read" "$output/write" "$output/setup" "$output/phases" "$work"

cargo test --release --manifest-path "$script_dir/Cargo.toml" \
    >"$output/correctness.txt" 2>&1
cargo test --release --manifest-path "$script_dir/Cargo.toml" \
    --test process_safety reader_writer_overlap_survives_one_hundred_rounds \
    -- --ignored --exact >>"$output/correctness.txt" 2>&1

sizes="100 1000 5000 10000 50000 100000"
schemas="sqlite-delete sqlite-delete-without-rowid"

for size in $sizes; do
    for backend in $schemas; do
        "$binary" init "$backend" "$work/${backend}-${size}" "$size"
    done
    for query in projects repo-000042 proj_src zzzzzznotfound; do
        case "$query" in
            proj_src) arguments="projects src" ;;
            *) arguments="$query" ;;
        esac
        hyperfine --shell=none --warmup 10 --runs 100 \
            --export-json "$output/read/${size}-${query}.json" \
            "$binary query sqlite-delete $work/sqlite-delete-${size} $arguments" \
            "$binary query sqlite-delete-without-rowid $work/sqlite-delete-without-rowid-${size} $arguments"
    done
done

for size in $sizes; do
    rowid_source="$work/sqlite-delete-${size}"
    rowid_target="$work/write-sqlite-delete-${size}"
    without_source="$work/sqlite-delete-without-rowid-${size}"
    without_target="$work/write-sqlite-delete-without-rowid-${size}"
    hyperfine --shell=none --warmup 5 --runs 100 \
        --prepare "$binary copy sqlite-delete $rowid_source $rowid_target" \
        --prepare "$binary copy sqlite-delete-without-rowid $without_source $without_target" \
        --export-json "$output/write/${size}-schema.json" \
        "$binary update sqlite-delete $rowid_target /benchmark/schema-followup" \
        "$binary update sqlite-delete-without-rowid $without_target /benchmark/schema-followup"
done

{
    printf 'records\tbackend\tbefore_bytes\tafter_vacuum_bytes\n'
    for size in $sizes; do
        for backend in $schemas; do
            source="$work/${backend}-${size}"
            target="$work/vacuum-${backend}-${size}"
            "$binary" copy "$backend" "$source" "$target"
            before=$("$binary" bytes "$backend" "$target")
            "$binary" vacuum "$backend" "$target"
            after=$("$binary" bytes "$backend" "$target")
            printf '%s\t%s\t%s\t%s\n' "$size" "$backend" "$before" "$after"
        done
    done
} >"$output/storage-size.tsv"

for size in 100 10000; do
    source="$work/sqlite-delete-without-rowid-${size}"
    hyperfine --shell=none --warmup 10 --runs 100 \
        --export-json "$output/setup/${size}-query.json" \
        "$binary query sqlite-delete-without-rowid $source projects" \
        "$binary query sqlite-delete-production $source projects"

    repeated_target="$work/setup-write-sqlite-delete-without-rowid-${size}"
    production_target="$work/setup-write-sqlite-delete-production-${size}"
    hyperfine --shell=none --warmup 5 --runs 100 \
        --prepare "$binary copy sqlite-delete-without-rowid $source $repeated_target" \
        --prepare "$binary copy sqlite-delete-production $source $production_target" \
        --export-json "$output/setup/${size}-update.json" \
        "$binary update sqlite-delete-without-rowid $repeated_target /benchmark/setup-followup" \
        "$binary update sqlite-delete-production $production_target /benchmark/setup-followup"

    for backend in sqlite-delete-without-rowid sqlite-delete-production; do
        for operation in query update; do
            phase_file="$output/phases/${size}-${backend}-${operation}.tsv"
            : >"$phase_file"
            iteration=0
            while [ "$iteration" -lt 100 ]; do
                if [ "$operation" = update ]; then
                    target="$work/phase-${backend}-${size}"
                    "$binary" copy "$backend" "$source" "$target"
                    ZFZ_TIMINGS=1 "$binary" update "$backend" "$target" \
                        /benchmark/setup-phase 2>>"$phase_file"
                else
                    ZFZ_TIMINGS=1 "$binary" query "$backend" "$source" projects \
                        >/dev/null 2>>"$phase_file"
                fi
                iteration=$((iteration + 1))
            done
        done
    done
done

for file in "$output"/read/*.json "$output"/write/*.json "$output"/setup/*.json; do
    jq -r -f "$script_dir/summarize.jq" "$file"
done >"$output/summary.tsv"

{
    date '+date=%Y-%m-%d'
    uname -a
    sw_vers
    rustc --version
    cargo --version
    hyperfine --version
    echo 'cache_condition=warm'
    echo 'sqlite_synchronous=FULL'
    echo 'scope=rowid schema, WITHOUT ROWID schema, and connection setup only'
} >"$output/environment.txt"
