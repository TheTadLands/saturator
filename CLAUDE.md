# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Saturator is a Rust benchmarking tool that measures CPU and I/O saturation points in containerized environments. It supports both thread-based and process-based experiments. Thread-based experiments show throughput plateaus; process-based experiments (with tunable work unit size and buffer size) demonstrate actual throughput degradation due to context switch overhead, TLB thrashing, and cache contention.

## Build & Run

```bash
# Build locally
cargo build --release

# Build and run via Docker (preferred - provides CPU/memory/network isolation)
docker compose run --build --remove-orphans saturator <experiment> [args] [OPTIONS]
```

There are no tests or lints configured.

### Experiments

**Thread-based:**
- `find-saturation` — incremental CPU-only threads until plateau
- `find-io-saturation` — incremental I/O-only threads until plateau
- `find-slack <N> <extra_io%>` — N CPU baseline threads + extra threads at given I/O ratio
- `find-io-slack <N> <extra_io%>` — N I/O baseline threads + extra threads at given I/O ratio

**Process-based** (separate address spaces via `fork+exec`, shared memory IPC):
- `find-saturation-proc` — incremental CPU-only processes until throughput degrades
- `find-io-saturation-proc` — incremental I/O-only processes until throughput degrades

**Tuning flags** (work with all experiments):
- `--target-us <N>` — calibration target per-op in μs (default: 50). Lower = more context switch sensitivity.
- `--buffer-kb <N>` — CPU work buffer size in KB (default: 100). Larger = more cache pressure.
- `--max-workers <N>` — max worker count (default: parallelism*4 for threads, parallelism*16 for procs)
- `--duration <N>` — measurement duration per data point in seconds (default: 5)
- `--samples <N>` — samples per data point; median and stddev computed (default: 3)
- `--step <N>` — worker count increment per data point (default: 1)

### Plotting

`plot_results.py` generates PNG visualizations from CSV output in `./output/`. Handles both `*throughput_vs_threads*` and `*throughput_vs_workers*` filenames. Plots include stddev error bars when the CSV contains stddev columns (backward-compatible with older CSVs).

## Architecture

Five source files in `src/`:

- **`main.rs`** — CLI argument parsing and experiment orchestration. Parses tuning flags into `TuningParams`, runs calibration, dispatches to experiment functions. Thread experiments use `Arc<AtomicU64>` for shared counters. Process experiments use `measure_single_run_proc` which creates named shared memory, spawns child processes via `Command::new(current_exe()).arg("__worker")`, and collects throughput. The hidden `__worker` subcommand is the child process entry point. All measurements compute median and stddev across configurable sample count (default 3). Stddev is written to CSV output for visualization as error bars.

- **`saturator.rs`** — Core workload engine and shared memory infrastructure. `TuningParams` controls calibration target, buffer size, max workers, and duration. Calibrates CPU and I/O operations using binary search (configurable target, default ~50μs). CPU work = hash computation over a configurable buffer. I/O work = read/write/seek with `O_SYNC`. `SharedRegion` is a `#[repr(C)]` struct with atomic counters laid out for cross-process mmap. Helpers `create_shared_region`/`open_shared_region`/`destroy_shared_region` use `libc::shm_open` + `libc::mmap(MAP_SHARED)`. `run_worker_process` is the child process work loop. Uses batched atomic counter updates (every 100 ops) to reduce contention.

- **`config.rs`** — Thread configuration structs (`ThreadGroupConfig`, `ExperimentConfig`) with JSON serialization and validation. Currently unused by CLI.

- **`visualize.rs`** — CSV writer for saturation and slack result types. Saturation CSV includes `throughput_stddev` column; slack CSV includes `cpu_ops_stddev` and `io_ops_stddev` columns.

- **`metrics.rs`** — Throughput monitor that samples atomic counters per-second. Currently unused.

## Key Design Details

- Thread-safe coordination via `Arc<AtomicU64>` counters and `Arc<AtomicBool>` shutdown signals (relaxed ordering).
- Process-based experiments use named POSIX shared memory (`/dev/shm`) for the same atomic counter pattern across process boundaries.
- 1-second warmup before each measurement; configurable measurement window (default 5s).
- Docker compose pins to CPU cores 4-7 (`cpuset` in `compose.yaml`), 8GB memory limit, no network, raised ulimits for high process counts. Adjust `cpuset` for your hardware.
- I/O scratch files go to `/tmp` (mapped to `./io_scratch` in Docker).
- Rust edition 2024; dependencies: `rand`, `libc`, `serde`, `serde_json`.
