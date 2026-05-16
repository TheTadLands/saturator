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

**SoI (Source of Interference)** sweep with synthetic victims:
- `find-soi-sweep <soi|all> <victim_workers> [victim_io%]` — hold fixed victim workers running a synthetic workload, sweep SoI workers that stress a specific shared resource (l1d, l2, l3, membw, memcap, cpu, iobw, ioops, or all). Measures victim throughput degradation as interference increases.
- `find-soi-phase-sweep <victim_workers> [victim_io%] --soi-phase-map 'type:io%,...'` — phase-matched SoI sweep. Multiple SoI types run simultaneously, each gated to activate only during a specific victim phase (requires `--victim-phases` and `--victim-period`). Each sweep step adds `--step` workers of every mapped type. Example: `--soi-phase-map 'cpu:100,l3:0'` activates CPU SoI during victim IO phase and L3 SoI during victim CPU phase.

**External workload** (real application as victim, SoI workers apply interference):
- `find-soi-sweep-ext <soi|all> --cmd '<command>' [OPTIONS]` — run an external workload (e.g. RocksDB's db_bench) as the victim, sweep SoI workers to measure interference. The external workload reports throughput via a cumulative file-based protocol (`<timestamp_ms> <cumulative_ops>\n`). Saturator computes rates from deltas between consecutive reports. Wrapper scripts adapt specific workloads to this protocol (see `scripts/run_db_bench.sh` for RocksDB).
- `find-saturation-ext <soi|all> --cmd '<cmd {N}>' [OPTIONS]` — find the saturation point of an external workload by sweeping the `{N}` template parameter (e.g. thread count). Optionally chain into SoI sweep at the saturation point with `--chain`.

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
- `--sample-interval <ms>` — enable time-series sampling during measurement windows (default: off, minimum: 100ms). When set, the parent process periodically reads per-worker atomic counters and cgroup metrics mid-flight, producing a `timeseries_soi_*.csv` alongside the aggregate CSV. Useful for observing phase-dependent interference effects in workloads with time-varying CPU/IO ratios.
- `--cmd <command>` — external workload command to run as victim (for `find-soi-sweep-ext`). Launched via `sh -c`. The `SATURATOR_THROUGHPUT_FILE` env var is set automatically.
- `--throughput-file <path>` — path for the external workload throughput protocol file (default: `/tmp/saturator_ext_throughput.txt`).
- `--stats-file <path>` — path for external workload stats protocol file (default: `/tmp/saturator_ext_stats.txt`). Wrapper-emitted rows are collected into `ext_stats_<soi_type>.csv`.
- `--nice <N>` — nice value for SoI workers (-20 to 19, default: inherit). Lower values give SoI workers higher scheduling priority.
- `--cooldown <N>` — idle gap in seconds between samples to let system state settle (default: 0).
- `--prefill <args>` — prefill command args for external workload (e.g. DB pre-population). Runs once before all samples via the wrapper's `SATURATOR_PREFILL` env var.
- `--soi-period <ms>` — SoI square-wave period in ms (default: off = continuous). When set, SoI workers alternate between active (on) and sleeping (off) phases. Minimum 10ms.
- `--soi-duty <F>` — SoI square-wave duty cycle 0.0–1.0, fraction of the period that is "on" (default: 0.5). Clamped to [0.01, 0.99]. Only meaningful when `--soi-period` is set.
- `--victim-period <ms>` — victim square-wave period in ms (default: off = continuous). When set without `--victim-phases`, victim workers alternate between active and sleeping phases (on/off gating). When set with `--victim-phases`, controls the total cycle time for phase cycling. Minimum 10ms. Works with all process-based experiments.
- `--victim-phases <io%,io%,...>` — comma-separated list of IO percentages (0–100) to cycle through (e.g., `0,100` for alternating pure CPU/IO). Each phase gets `period / N` time. Requires `--victim-period`.
- `--soi-phase-map <type:io%,...>` — map SoI types to victim phases for `find-soi-phase-sweep`. Each entry is `soi_type:victim_io_pct` (e.g., `cpu:100,l3:0`). SoI workers self-gate by reading `victim_io_perc` from shared memory and sleeping when the current phase doesn't match.

### Plotting

`plot_results.py` generates PNG visualizations from CSV output in `./output/`. Handles both `*throughput_vs_threads*` and `*throughput_vs_workers*` filenames. Plots include stddev error bars when the CSV contains stddev columns (backward-compatible with older CSVs).

### Per-Worker CSV Output

Process-based experiments (`-proc` variants) produce a per-worker throughput CSV alongside the aggregate CSV. This reveals fairness/starvation issues across workers. Files are named `per_worker_<csv_base>.csv` for saturation experiments and `per_worker_proc_intensity_sweep_*.csv` for intensity sweeps. Format: `workers,worker_id,cpu_ops_sec,io_ops_sec,total_ops_sec` (intensity sweep adds `probe_intensity` column). Thread-mode experiments do not produce per-worker CSVs.

## Architecture

Source files in `src/`:

- **`main.rs`** — CLI argument parsing, `__worker` child process dispatch, and experiment dispatch via match. Parses tuning flags into `TuningParams`, runs calibration, then delegates to experiment functions.

- **`constants.rs`** — Named constants for magic numbers used throughout: RNG parameters (PCG multiplier, seed offset), CPU work parameters (buffer stride, hash multipliers, unroll factor), calibration sample counts and tolerances, shared memory layout sizes, and batch flush threshold.

- **`saturator.rs`** — Core workload engine and shared memory infrastructure. `TuningParams` controls calibration target, buffer size, max workers, duration, intensity, random access pattern, and direct I/O mode. Calibrates CPU and I/O operations using binary search (configurable target, default ~50μs). CPU work = hash computation over a configurable buffer (sequential stride by default, hash-derived random offsets with `--random-access`). I/O work = write with `O_SYNC` (or `O_DIRECT | O_SYNC` with `--direct-io`; uses `posix_memalign` for aligned buffers). `SharedRegion` is a `#[repr(C)]` struct with atomic counters laid out for cross-process mmap. Includes `soi_gate` and `victim_gate` fields for independent square-wave gating (1=on, 0=off), each toggled by a coordinator thread via `spawn_soi_gate_coordinator()`. Per-worker counter slots are cache-line-aligned (64 bytes each) to eliminate false sharing. Workers only write to their own per-worker slot during measurement; aggregate totals are summed from per-worker counters after measurement. Helpers `create_shared_region`/`open_shared_region`/`destroy_shared_region` use `libc::shm_open` + `libc::mmap(MAP_SHARED)`. `run_worker_process` is the child process work loop.

- **`experiments/`** — Experiment orchestration, one file per experiment type:
  - `mod.rs` — `Mode` enum (Threads/Procs), `SaturationExperiment` and `SlackExperiment` config structs.
  - `saturation.rs` — `run_saturation_experiment()`: incremental workers until throughput plateaus/degrades.
  - `slack.rs` — `run_slack_experiment()`: baseline threads + extra threads at different I/O ratios.
  - `intensity.rs` — `run_intensity_sweep_experiment()`: sweep probe worker intensity 0.0–1.0.
  - `slack_proc.rs` — `run_slack_proc_experiment()`: process-based slack with baseline + extra workers.
  - `soi_sweep.rs` — `run_soi_sweep_experiment()`, `run_soi_experiments()`: SoI sweep with synthetic victim workers.
  - `soi_phase_sweep.rs` — `run_soi_phase_sweep_experiment()`: phase-matched SoI sweep with multiple SoI types gated to specific victim phases.
  - `soi_sweep_ext.rs` — `run_soi_sweep_ext_experiment()`, `run_soi_ext_experiments()`, `run_ext_saturation_and_sweep()`: SoI sweep with an external workload as the victim instead of synthetic workers.

- **`measure/`** — Measurement infrastructure:
  - `mod.rs` — Shared utilities: `median()`, `stddev()`, `timestamp()`, `write_params_file()`, `cleanup_scratch_files()`, `aggregate_samples()`, `TimeSeriesSample` struct.
  - `thread.rs` — Thread-based measurements: `measure_single_run()`, `measure_thread_throughput()`, `measure_baseline()`, `measure_total_throughput()`, `run_saturator_split()`.
  - `proc.rs` — Process-based measurements: `measure_single_run_proc()`, `measure_proc_throughput()`, `measure_single_run_proc_mixed_intensity()`, `measure_proc_slack()`, `spawn_soi_worker()`, `measure_single_run_soi_phased()`, `measure_soi_phased_throughput()`. When `sample_interval_ms` is set, `run_mixed_proc_measurement()` replaces its single sleep with a polling loop that reads per-worker atomics and cgroup snapshots at each interval.
  - `ext.rs` — External workload measurements: `run_ext_measurement()`, `measure_ext_throughput()`, `run_prefill_blocking()`. Launches an external command as the victim alongside SoI workers. SoI workers use shared memory; external workload reports throughput via file protocol.
  - `ext_throughput.rs` — `ThroughputReader`: reads the external workload cumulative throughput protocol file and computes instantaneous rates from deltas. Tracks cumulative state across calls; `reset()` seeds state from the last warmup line. Designed to be swappable for different throughput reporting mechanisms.
  - `ext_stats.rs` — `StatsRow` struct and helpers (`truncate()`, `read_all()`) for reading workload-specific stats emitted by external wrapper scripts via the stats protocol file.

- **`visualize.rs`** — CSV writer for saturation result types. Saturation CSV includes `throughput_stddev` column; slack CSV includes `cpu_ops_stddev` and `io_ops_stddev` columns.

- **`soi.rs`** — SoI (Source of Interference) worker types and infrastructure. Defines `SoiType` enum (L1d, L2, L3, MemBw, MemCap, Cpu, IoBw, IoOps), `SoiBackend` enum (Builtin/Fio), `CacheSizes` struct, `parse_soi_list()`, `detect_cache_sizes()`, `soi_buffer_size()`, and `run_soi_worker_process()` (child process entry point for SoI workers). SoI work functions support phase-gating via `PhaseGate` — when set, workers self-gate by comparing the current `victim_io_perc` from shared memory against their assigned active phase.

- **`proc_metrics.rs`** — System metrics collector. Reads cgroup CPU/IO stats and PSI pressure to compute CPU utilization, IO bandwidth utilization, and pressure stall percentages during measurement windows. Provides `SystemMetrics` struct and CSV output helpers. When `--perf` is enabled, `SystemMetrics` carries an `Option<PerfMetrics>` that flows through all CSV output automatically.

- **`perf_counters.rs`** — Hardware performance counter collection via `perf_event_open` syscall. Always enabled; gracefully degrades to no-op if counters are unavailable (e.g. no `CAP_PERFMON`). Opens counters cgroup-scoped across all CPUs in the cpuset. Global singleton pattern: `open()` initializes once, `read_snapshot()` reads current counts (called from `proc_metrics::take_snapshot()`), `close()` releases fds. Zero overhead during measurement — hardware registers count autonomously, only two `read()` syscalls per counter per measurement boundary. When available, adds 8 columns to CSV: `l1d_load_misses`, `llc_load_misses`, `cache_misses`, `instructions`, `cycles`, `ipc`, `l1d_miss_per_kinsn`, `llc_miss_per_kinsn`.

## External Submodules

- **`iBench/`** — Git submodule containing an external benchmarking tool. Do not modify code in this directory unless explicitly directed to.

## Key Design Details

- Thread-safe coordination via `Arc<AtomicU64>` counters and `Arc<AtomicBool>` shutdown signals (relaxed ordering).
- Process-based experiments use named POSIX shared memory (`/dev/shm`) for the same atomic counter pattern across process boundaries. Per-worker slots are padded to 64 bytes (one cache line) to eliminate false sharing; workers never write to global counters during measurement.
- Configurable warmup before each measurement (default 10s, tunable via `--warmup`); configurable measurement window (default 30s, tunable via `--duration`).
- Docker compose pins to CPU cores 4-7 (`cpuset` in `compose.yaml`), 8GB memory limit, no network, raised ulimits for high process counts. Adjust `cpuset` for your hardware.
- I/O scratch files go to `/tmp` (mapped to `./io_scratch` in Docker).
- Rust edition 2024; dependencies: `libc`.
