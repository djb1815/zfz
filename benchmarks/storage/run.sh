#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 OUTPUT_DIRECTORY" >&2
    exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/../.." && pwd)
binary="$script_dir/target/release/zfz-storage-benchmark"
output=$1
work="$output/work"

test -x "$binary"
command -v hyperfine >/dev/null
command -v jq >/dev/null
mkdir -p "$output/read" "$output/write" "$output/replay" "$output/phases" "$work"

cargo test --manifest-path "$script_dir/Cargo.toml" >"$output/correctness.txt" 2>&1
cargo test --release --manifest-path "$script_dir/Cargo.toml" \
    --test process_safety reader_writer_overlap_survives_one_hundred_rounds \
    -- --ignored --exact >>"$output/correctness.txt" 2>&1

sizes="100 1000 5000 10000 50000 100000"
backends="journal sqlite-delete sqlite-wal"

for size in $sizes; do
    for backend in $backends; do
        "$binary" init "$backend" "$work/${backend}-${size}" "$size"
    done
    for query in projects repo-000042 proj_src zzzzzznotfound; do
        case "$query" in
            proj_src) arguments="projects src" ;;
            *) arguments="$query" ;;
        esac
        hyperfine --shell=none --warmup 10 --runs 100 \
            --export-json "$output/read/${size}-${query}.json" \
            "$binary query journal $work/journal-${size} $arguments" \
            "$binary query sqlite-delete $work/sqlite-delete-${size} $arguments" \
            "$binary query sqlite-wal $work/sqlite-wal-${size} $arguments"
    done
done

for size in 1000 100000; do
    for backend in $backends; do
        hyperfine --shell=none --warmup 1 --runs 30 \
            --prepare "$binary copy $backend $work/${backend}-${size} $work/fresh-${backend}-${size}" \
            --export-json "$output/read/fresh-${size}-${backend}.json" \
            "$binary query $backend $work/fresh-${backend}-${size} projects"
    done
done

for size in $sizes; do
    for backend in $backends; do
        source="$work/${backend}-${size}"
        target="$work/write-${backend}-${size}"
        hyperfine --shell=none --warmup 5 --runs 100 \
            --prepare "$binary copy $backend $source $target" \
            --export-json "$output/write/${size}-${backend}-single.json" \
            "$binary update $backend $target /benchmark/single"
    done
done

for size in 1000 100000; do
    for burst in 10 100 1000; do
        case "$burst" in
            10) runs=30 ;;
            100) runs=10 ;;
            *) runs=3 ;;
        esac
        for backend in $backends; do
            source="$work/${backend}-${size}"
            target="$work/burst-${backend}-${size}"
            hyperfine --shell=none --warmup 1 --runs "$runs" \
                --prepare "$binary copy $backend $source $target" \
                --export-json "$output/write/${size}-${backend}-burst-${burst}.json" \
                "$binary burst $backend $target $burst 16"
        done
    done
done

for entries in 0 10 100 1000 10000; do
    store="$work/replay-${entries}"
    "$binary" init journal "$store" 10000
    if [ "$entries" -gt 0 ]; then
        "$binary" burst journal "$store" "$entries" 128
    fi
    hyperfine --shell=none --warmup 5 --runs 100 \
        --export-json "$output/replay/load-${entries}.json" \
        "$binary query journal $store projects"
    hyperfine --shell=none --warmup 1 --runs 30 \
        --prepare "$binary copy journal $store $work/compact-${entries}" \
        --export-json "$output/replay/compact-${entries}.json" \
        "$binary compact journal $work/compact-${entries}"
done

for size in $sizes; do
    for backend in $backends; do
        for operation in query update; do
            phase_file="$output/phases/${size}-${backend}-${operation}.tsv"
            : > "$phase_file"
            iteration=0
            while [ "$iteration" -lt 100 ]; do
                if [ "$operation" = update ]; then
                    "$binary" copy "$backend" "$work/${backend}-${size}" "$work/phase-${backend}-${size}"
                    ZFZ_TIMINGS=1 "$binary" update "$backend" "$work/phase-${backend}-${size}" /benchmark/phase 2>>"$phase_file"
                else
                    ZFZ_TIMINGS=1 "$binary" query "$backend" "$work/${backend}-${size}" projects >/dev/null 2>>"$phase_file"
                fi
                iteration=$((iteration + 1))
            done
        done
    done
done

for entries in 0 10000; do
    phase_file="$output/phases/compact-${entries}.tsv"
    : > "$phase_file"
    iteration=0
    while [ "$iteration" -lt 100 ]; do
        "$binary" copy journal "$work/replay-${entries}" "$work/profile-compact-${entries}"
        ZFZ_TIMINGS=1 "$binary" compact journal "$work/profile-compact-${entries}" 2>>"$phase_file"
        iteration=$((iteration + 1))
    done
done

for file in "$output"/read/*.json "$output"/write/*.json "$output"/replay/*.json; do
    jq -r -f "$script_dir/summarize.jq" "$file"
done > "$output/summary.tsv"

sqlite_binary_bytes=$(stat -f '%z' "$binary")
cargo build --quiet --release --no-default-features --manifest-path "$script_dir/Cargo.toml"
snapshot_binary_bytes=$(stat -f '%z' "$binary")
cargo build --quiet --release --manifest-path "$script_dir/Cargo.toml"

{
    date '+date=%Y-%m-%d'
    uname -a
    sw_vers
    system_profiler SPHardwareDataType | sed -n \
        -e '/Model Name:/p' -e '/Model Identifier:/p' -e '/Chip:/p' \
        -e '/Total Number of Cores:/p' -e '/Memory:/p'
    mount | sed -n '/\/Volumes\/Projects /p'
    rustc --version
    cargo --version
    hyperfine --version
    stat -f 'production_binary_bytes=%z' "$repo_dir/target/release/zfz"
    echo "snapshot_only_benchmark_binary_bytes=$snapshot_binary_bytes"
    echo "sqlite_benchmark_binary_bytes=$sqlite_binary_bytes"
    echo "cache_condition=warm; fresh-instance samples are not claimed as cold"
} > "$output/environment.txt"

for size in $sizes; do
    for backend in $backends; do
        printf '%s\t%s\t%s\n' "$size" "$backend" \
            "$("$binary" bytes "$backend" "$work/${backend}-${size}")"
    done
done > "$output/storage-size.tsv"
