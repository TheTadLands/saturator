# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Documentation

- **`readme.md`** — User-facing documentation with experiment descriptions, CLI usage, CSV column references, and configuration details. Keep in sync when adding/changing experiments or flags.

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
- `find-mixed-saturation-proc <io_pct>` — incremental mixed CPU+IO processes at given IO% (0-100) until throughput degrades
- `find-saturation-intensity-proc <N> <io_pct>` — N base processes at intensity=1.0 + 1 probe process, sweep probe intensity 0.0–1.0 to find saturation tipping point

**Tuning flags** (work with all experiments):
- `--buffer-kb <N>` — CPU work buffer size in KB (default: 100). Larger = more cache pressure.
- `--io-buffer-kb <N>` — IO read/write buffer size in KB (default: 4). Larger = more bytes per IO op, higher bandwidth utilization.
- `--max-workers <N>` — max worker count (default: parallelism*4 for threads, parallelism*16 for procs)
- `--duration <N>` — measurement duration per data point in seconds (default: 30)
- `--samples <N>` — samples per data point; median and stddev computed (default: 5)
- `--step <N>` — worker count increment per data point (default: 1)
- `--intensity <F>` — work probability per iteration, 0.0–1.0 (default: 1.0). Idle iterations sleep for `target_us` μs.
- `--chain` — after a proc saturation experiment finds the saturation point N, automatically run `find-saturation-intensity-proc` with N base workers.
- `--warmup <N>` — warmup duration in seconds before each measurement (default: 10). Longer warmup useful for cold caches or heavy I/O workloads.
- `--random-access` — use hash-derived random buffer offsets for CPU work instead of sequential stride. Defeats hardware prefetcher so cache misses scale with buffer size.
- `--direct-io` — use `O_DIRECT | O_SYNC` for I/O writes, bypassing the page cache. Each write is a full round-trip to the block device. Requires page-aligned buffers (handled automatically via `posix_memalign`).

### Plotting

`plot_results.py` generates PNG visualizations from CSV output in `./output/`. Handles both `*throughput_vs_threads*` and `*throughput_vs_workers*` filenames. Plots include stddev error bars when the CSV contains stddev columns (backward-compatible with older CSVs).

### Per-Worker CSV Output

Process-based experiments (`-proc` variants) produce a per-worker throughput CSV alongside the aggregate CSV. This reveals fairness/starvation issues across workers. Files are named `per_worker_<csv_base>.csv` for saturation experiments and `per_worker_proc_intensity_sweep_*.csv` for intensity sweeps. Format: `workers,worker_id,cpu_ops_sec,io_ops_sec,total_ops_sec` (intensity sweep adds `probe_intensity` column). Thread-mode experiments do not produce per-worker CSVs.

## Architecture

Source files in `src/`:

- **`main.rs`** — CLI argument parsing, `__worker` child process dispatch, and experiment dispatch via match. Parses tuning flags into `TuningParams`, runs calibration, then delegates to experiment functions.

- **`constants.rs`** — Named constants for magic numbers used throughout: RNG parameters (PCG multiplier, seed offset), CPU work parameters (buffer stride, hash multipliers, unroll factor), calibration sample counts and tolerances, shared memory layout sizes, and batch flush threshold.

- **`saturator.rs`** — Core workload engine and shared memory infrastructure. `TuningParams` controls calibration target, buffer size, max workers, duration, intensity, random access pattern, and direct I/O mode. Calibrates CPU and I/O operations using binary search (configurable target, default ~50μs). CPU work = hash computation over a configurable buffer (sequential stride by default, hash-derived random offsets with `--random-access`). I/O work = write with `O_SYNC` (or `O_DIRECT | O_SYNC` with `--direct-io`; uses `posix_memalign` for aligned buffers). `SharedRegion` is a `#[repr(C)]` struct with atomic counters laid out for cross-process mmap. Per-worker counter slots are cache-line-aligned (64 bytes each) to eliminate false sharing. Workers only write to their own per-worker slot during measurement; aggregate totals are summed from per-worker counters after measurement. Helpers `create_shared_region`/`open_shared_region`/`destroy_shared_region` use `libc::shm_open` + `libc::mmap(MAP_SHARED)`. `run_worker_process` is the child process work loop.

- **`experiments/`** — Experiment orchestration, one file per experiment type:
  - `mod.rs` — `Mode` enum (Threads/Procs), `SaturationExperiment` and `SlackExperiment` config structs.
  - `saturation.rs` — `run_saturation_experiment()`: incremental workers until throughput plateaus/degrades.
  - `slack.rs` — `run_slack_experiment()`: baseline threads + extra threads at different I/O ratios.
  - `intensity.rs` — `run_intensity_sweep_experiment()`: sweep probe worker intensity 0.0–1.0.
  - `slack_proc.rs` — `run_slack_proc_experiment()`: process-based slack with baseline + extra workers.

- **`measure/`** — Measurement infrastructure:
  - `mod.rs` — Shared utilities: `median()`, `stddev()`, `timestamp()`, `write_params_file()`, `cleanup_scratch_files()`, `aggregate_samples()`.
  - `thread.rs` — Thread-based measurements: `measure_single_run()`, `measure_thread_throughput()`, `measure_baseline()`, `measure_total_throughput()`, `run_saturator_split()`.
  - `proc.rs` — Process-based measurements: `measure_single_run_proc()`, `measure_proc_throughput()`, `measure_single_run_proc_mixed_intensity()`, `measure_proc_slack()`.

- **`visualize.rs`** — CSV writer for saturation result types. Saturation CSV includes `throughput_stddev` column; slack CSV includes `cpu_ops_stddev` and `io_ops_stddev` columns.

- **`proc_metrics.rs`** — System metrics collector. Reads cgroup CPU/IO stats and PSI pressure to compute CPU utilization, IO bandwidth utilization, and pressure stall percentages during measurement windows. Provides `SystemMetrics` struct and CSV output helpers.

## External Submodules

- **`iBench/`** — Git submodule containing an external benchmarking tool. Do not modify code in this directory unless explicitly directed to.

## Key Design Details

- Thread-safe coordination via `Arc<AtomicU64>` counters and `Arc<AtomicBool>` shutdown signals (relaxed ordering).
- Process-based experiments use named POSIX shared memory (`/dev/shm`) for the same atomic counter pattern across process boundaries. Per-worker slots are padded to 64 bytes (one cache line) to eliminate false sharing; workers never write to global counters during measurement.
- Configurable warmup before each measurement (default 1s, tunable via `--warmup`); configurable measurement window (default 5s).
- Docker compose pins to CPU cores 4-7 (`cpuset` in `compose.yaml`), 8GB memory limit, no network, raised ulimits for high process counts. Adjust `cpuset` for your hardware.
- I/O scratch files go to `/tmp` (mapped to `./io_scratch` in Docker).
- Rust edition 2024; dependencies: `libc`.
