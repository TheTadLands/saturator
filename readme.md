# Saturator

A tool for measuring CPU and I/O saturation points and finding slack capacity in containerized environments. Supports both thread-based and process-based experiments with configurable workload parameters.

## Purpose

Saturator helps answer:
1. Where is the saturation point (number of processes) for a given workload?
2. How many processes with a given workload can we add before performance degrades?
3. Can we model our our system's slack with system tools?

Workload parameters are tunable: CPU buffer size (`--buffer-kb`), I/O buffer size (`--io-buffer-kb`), worker intensity (`--intensity`), random access patterns (`--random-access`), direct I/O (`--direct-io`), and measurement duration/samples. This lets you control how much cache pressure, I/O bandwidth, and context switch overhead each worker generates.

## Running

```bash
docker compose run --build --remove-orphans saturator <experiment> [args] [OPTIONS]
```

- `--build` ensures you're running the latest code
- `--remove-orphans` cleans up old containers

## Experiments

### Thread-Based

#### Find CPU Saturation
Incrementally adds CPU-only threads until throughput plateaus.

```bash
docker compose run --build --remove-orphans saturator find-saturation
```

**Output:** `cpu_throughput_vs_threads.csv`

#### Find I/O Saturation
Incrementally adds I/O-only threads until throughput plateaus.

```bash
docker compose run --build --remove-orphans saturator find-io-saturation
```

**Output:** `io_throughput_vs_threads.csv`

#### Find Slack (CPU Baseline)
Starts with N CPU-only baseline threads, then adds threads at a specified I/O percentage.

```bash
docker compose run --build --remove-orphans saturator find-slack <N> <extra_io%>
```

Arguments:
- `N` - Number of CPU-only baseline threads
- `extra_io%` - I/O percentage for extra threads being added

Examples:
```bash
# 4 CPU-only baseline threads, add I/O-only threads
docker compose run --build --remove-orphans saturator find-slack 4 100

# 4 CPU-only baseline threads, add 50% I/O threads
docker compose run --build --remove-orphans saturator find-slack 4 50

# 4 CPU-only baseline threads, add CPU-only threads
docker compose run --build --remove-orphans saturator find-slack 4 0
```

**Output:** `slack_<N>cpu_adding_<io%>pct_io.csv`

#### Find I/O Slack (I/O Baseline)
Starts with N I/O-only baseline threads, then adds threads at a specified I/O percentage.

```bash
docker compose run --build --remove-orphans saturator find-io-slack <N> <extra_io%>
```

Examples:
```bash
docker compose run --build --remove-orphans saturator find-io-slack 4 0
docker compose run --build --remove-orphans saturator find-io-slack 4 50
docker compose run --build --remove-orphans saturator find-io-slack 4 100
```

**Output:** `slack_<N>io_adding_<io%>pct_io.csv`

### Process-Based

Process-based experiments spawn separate OS processes instead of threads. Each process has its own address space, page tables, and TLB entries, which makes context switch overhead, cache thrashing, and TLB pressure visible at high worker counts — demonstrating actual throughput degradation rather than just a plateau.

#### Find CPU Saturation (Processes)
```bash
docker compose run --build --remove-orphans saturator find-saturation-proc
```

**Output:** `proc_cpu_throughput_vs_workers.csv`, `per_worker_proc_cpu_throughput_vs_workers.csv`

#### Find I/O Saturation (Processes)
```bash
docker compose run --build --remove-orphans saturator find-io-saturation-proc
```

**Output:** `proc_io_throughput_vs_workers.csv`, `per_worker_proc_io_throughput_vs_workers.csv`

#### Find Mixed Saturation (Processes)
Incrementally adds processes running a mix of CPU and I/O work at a given ratio until throughput degrades.

```bash
docker compose run --build --remove-orphans saturator find-mixed-saturation-proc <io_pct>
```

**Output:** `proc_mixed_<io_pct>pct_io_throughput_vs_workers.csv`, `per_worker_proc_mixed_<io_pct>pct_io_throughput_vs_workers.csv`

#### Find Saturation Intensity (Processes)
Holds N base processes at full intensity and sweeps a single probe process from 0.0 to 1.0 intensity, finding the point at which the probe tips the system into degradation.

```bash
docker compose run --build --remove-orphans saturator find-saturation-intensity-proc <N> <io_pct>
```

**Output:** `proc_intensity_sweep_<N>base_<io_pct>pct_io.csv`, `per_worker_proc_intensity_sweep_<N>base_<io_pct>pct_io.csv`

#### Find Slack (CPU Baseline, Processes)
Holds N CPU-only baseline processes at full intensity, then incrementally adds extra processes at a specified I/O percentage. Tracks baseline throughput separately from extra worker throughput, revealing how much the baseline degrades as contention increases.

```bash
docker compose run --build --remove-orphans saturator find-slack-proc <N> <extra_io%>
```

**Output:** `proc_slack_<N>cpuproc_adding_<io%>pct_io.csv`, `per_worker_proc_slack_<N>cpuproc_adding_<io%>pct_io.csv`

#### Find Slack (I/O Baseline, Processes)
Same as above but with an I/O-only baseline.

```bash
docker compose run --build --remove-orphans saturator find-io-slack-proc <N> <extra_io%>
```

**Output:** `proc_slack_<N>ioproc_adding_<io%>pct_io.csv`, `per_worker_proc_slack_<N>ioproc_adding_<io%>pct_io.csv`

#### Find SoI Slack (Source of Interference)
Holds a fixed number of victim workers running a real workload, then incrementally adds SoI (Source of Interference) workers that stress a specific shared resource. Measures how victim throughput degrades as interference increases.

```bash
docker compose run --build --remove-orphans saturator find-soi-slack <soi|all> <victim_workers> [victim_io%] [OPTIONS]
```

Arguments:
- `soi` — SoI type to sweep: `l1d`, `l2`, `l3`, `membw`, `memcap`, `cpu`, `iobw`, `ioops`, or `all` to run every type
- `victim_workers` — Number of fixed victim workers
- `victim_io%` — IO percentage for victim workers (default: 0, i.e. CPU-only)

Examples:
```bash
# 32 I/O victim workers, sweep IOPS interference
docker compose run --build --remove-orphans saturator find-soi-slack ioops 32 100 --max-workers 16

# 4 CPU victim workers, sweep all SoI types
docker compose run --build --remove-orphans saturator find-soi-slack all 4 0 --duration 5 --samples 3
```

**Output:** `soi_<type>_<N>_victims_<io%>pct_io_<timestamp>/soi_<type>_throughput.csv`, `per_worker_soi_<type>.csv`

### Tuning Options

All experiments accept optional flags to control workload parameters.

| Flag | Default | Description |
|------|---------|-------------|
| `--buffer-kb <N>` | 100 | CPU work buffer size in KB. Larger values (e.g. 1024-10240) cause L3 cache contention at high worker counts. |
| `--io-buffer-kb <N>` | 4 | IO read/write buffer size in KB. Larger values (e.g. 64-1024) increase bytes per IO operation, driving higher bandwidth utilization. |
| `--max-workers <N>` | parallelism * 16 | Maximum number of workers to test. For proc slack experiments, controls the maximum number of extra workers added. |
| `--duration <N>` | 30 | Measurement duration per data point in seconds. |
| `--samples <N>` | 5 | Samples per data point. Median and stddev are computed; stddev is written to CSV for error bar visualization. |
| `--step <N>` | 1 | Worker count increment per data point. |
| `--intensity <F>` | 1.0 | Work probability per iteration (0.0–1.0). Each idle iteration sleeps instead of working. Simulates partially-loaded workers. |
| `--warmup <N>` | 10 | Warmup duration in seconds before each measurement. |
| `--random-access` | — | Use hash-derived random buffer offsets for CPU work instead of sequential stride. Defeats hardware prefetcher so cache misses scale with buffer size. |
| `--direct-io` | — | Use `O_DIRECT \| O_SYNC` for I/O writes, bypassing the page cache. Each write is a full round-trip to the block device. |
| `--chain` | — | After a proc saturation experiment finds the saturation point N, automatically runs `find-saturation-intensity-proc` with N base workers. |
| `--sample-interval <ms>` | off | Enable time-series sampling during measurement windows (minimum: 100ms). Produces a `timeseries_soi_*.csv` with per-interval throughput and utilization traces. See [Time-Series CSV columns](#time-series-csv-columns). |

Examples:
```bash
# Large buffers to maximize cache pressure
docker compose run --build --remove-orphans saturator find-saturation-proc \
    --buffer-kb 1024 --max-workers 100

# Compare threads vs processes with same parameters
docker compose run --build --remove-orphans saturator find-saturation \
    --max-workers 64
docker compose run --build --remove-orphans saturator find-saturation-proc \
    --max-workers 64

# Find proc slack: 4 CPU baseline processes, add I/O-only processes
docker compose run --build --remove-orphans saturator find-slack-proc 4 100

# Chain saturation into intensity sweep automatically
docker compose run --build --remove-orphans saturator find-saturation-proc --chain
```

## Output

Each experiment run creates a timestamped directory inside `./output/` containing CSV files and a `params.txt` recording the exact parameters used.

### Saturation CSV columns

Produced by `find-saturation`, `find-io-saturation`, `find-saturation-proc`, `find-io-saturation-proc`, and `find-mixed-saturation-proc`.

| Column | Description |
|--------|-------------|
| `threads` | Number of threads or processes |
| `cpu_ops_sec` | CPU operations per second across all workers |
| `io_ops_sec` | IO operations per second across all workers |
| `total_ops_sec` | Total operations per second (cpu + io) |
| `throughput_per_thread` | Total operations per second per worker |
| `cpu_ops_stddev` | Standard deviation of CPU ops across samples |
| `io_ops_stddev` | Standard deviation of IO ops across samples |
| `cpu_pct` | Container CPU utilization (cgroup-scoped, 100% = all assigned cores busy) |
| `system_pct` | Kernel/system CPU time as percentage of available CPU |
| `io_errors` | Count of failed IO operations across all workers |
| `io_util_pct` | IO bandwidth utilization as percentage of cgroup limit (from `io.stat` / `io.max`) |
| `io_iops_util_pct` | IO operations utilization as percentage of cgroup IOPS limit (from `io.stat` / `io.max`) |
| `io_psi_pct` | IO pressure — percentage of wall time at least one task was stalled on IO |
| `psi_cpu_some_us` | PSI CPU "some" stall time in microseconds |
| `psi_io_some_us` | PSI IO "some" stall time in microseconds |
| `psi_io_full_us` | PSI IO "full" stall time in microseconds |

### Slack CSV columns

Produced by `find-slack` and `find-io-slack`.

| Column | Description |
|--------|-------------|
| `extra_threads` | Number of extra threads added beyond baseline |
| `total_threads` | Total thread count (baseline + extra) |
| `extra_io_pct` | IO percentage of the extra threads |
| `cpu_ops` | CPU operations per second |
| `io_ops` | IO operations per second |
| `total_ops` | Total operations per second (cpu_ops + io_ops) |
| `baseline_change_pct` | Percentage change in tracked metric vs baseline |
| `cpu_ops_stddev` | Standard deviation of CPU ops across samples |
| `io_ops_stddev` | Standard deviation of IO ops across samples |
| `cpu_pct` ... `psi_io_full_us` | Same system metrics as saturation CSV |

### Proc Slack CSV columns

Produced by `find-slack-proc` and `find-io-slack-proc`. Baseline and extra worker throughputs are tracked separately using per-worker counters.

| Column | Description |
|--------|-------------|
| `extra_workers` | Number of extra processes added beyond baseline |
| `total_workers` | Total process count (baseline + extra) |
| `baseline_workers` | Fixed number of baseline processes |
| `extra_io_pct` | IO percentage of the extra processes |
| `baseline_cpu_ops` | CPU ops/sec summed across baseline workers only |
| `baseline_io_ops` | IO ops/sec summed across baseline workers only |
| `extra_cpu_ops` | CPU ops/sec summed across extra workers only |
| `extra_io_ops` | IO ops/sec summed across extra workers only |
| `total_ops` | Total ops/sec across all workers |
| `baseline_change_pct` | Percentage change in baseline tracked metric vs alone |
| `baseline_cpu_stddev` | Standard deviation of baseline CPU ops across samples |
| `baseline_io_stddev` | Standard deviation of baseline IO ops across samples |
| `cpu_pct` ... `psi_io_full_us` | Same system metrics as saturation CSV |

### Per-Worker CSVs

Process-based experiments produce a `per_worker_*.csv` alongside the aggregate CSV. This reveals fairness and starvation issues across individual workers.

| Experiment type | Columns |
|----------------|---------|
| Saturation (`-proc`) | `workers, worker_id, cpu_ops_sec, io_ops_sec, sleep_ops_sec, total_ops_sec` |
| Intensity sweep | `probe_intensity, workers, worker_id, cpu_ops_sec, io_ops_sec, sleep_ops_sec, total_ops_sec` |
| Proc slack | `extra_workers, total_workers, baseline_workers, worker_id, cpu_ops_sec, io_ops_sec, sleep_ops_sec, total_ops_sec` |

### Time-Series CSV columns

Produced by SoI sweep experiments when `--sample-interval` is set. One row per sampling interval per sweep step. File: `timeseries_soi_<type>.csv`.

| Column | Description |
|--------|-------------|
| `soi_workers` | Number of SoI workers active during this sweep step |
| `elapsed_ms` | Milliseconds since start of the measurement window |
| `victim_cpu_ops_sec` | Victim CPU operations per second during this interval |
| `victim_io_ops_sec` | Victim IO operations per second during this interval |
| `soi_ops_sec` | SoI worker operations per second during this interval |
| `cpu_pct` ... `psi_io_full_us` | Same system metrics as saturation CSV, computed over the interval |

Example:
```bash
# 1-second sampling with 10s measurement windows
docker compose run --build --remove-orphans saturator find-soi-slack cpu 4 50 \
    --sample-interval 1000 --duration 10 --samples 1
```

### Plotting

```bash
python plot_results.py              # Plot all CSVs in ./output
python plot_results.py output/run/  # Plot a specific run directory
python plot_results.py path/to/file.csv # Plot a specific file
```

Generates PNG visualizations alongside each CSV. Each run gets its own timestamped directory, so identically-named PNGs from different experiment types never collide:

| File | Produced by | Description |
|------|-------------|-------------|
| `throughput_total.png` | Saturation | CPU, IO, and total ops/sec vs worker count with stddev error bars |
| `throughput_per_worker.png` | Saturation | Per-worker efficiency vs worker count |
| `utilization.png` | Saturation, Slack | CPU% and IO% vs worker/thread count |
| `throughput_vs_cpu.png` | Saturation | Dual-axis: total throughput and CPU utilization |
| `throughput_vs_utilization.png` | Saturation, Slack | Normalized throughput % of peak vs utilization % |
| `throughput.png` | Slack | CPU and IO ops/sec vs extra thread count (dual y-axes) |
| `baseline_change.png` | Slack | Baseline throughput degradation % vs extra thread count |
| `per_worker_distribution.png` | Saturation (proc) | Box plot of per-worker throughput distribution at each worker count |
| `per_worker_fairness.png` | Saturation (proc) | Total throughput vs coefficient of variation (fairness) |
| `per_worker_intensity.png` | Intensity sweep | Base worker box plots vs probe worker throughput line |
| `throughput.png` | Proc slack | Baseline and extra worker throughputs vs extra worker count |
| `baseline_change.png` | Proc slack | Baseline throughput degradation % vs extra worker count |
| `utilization.png` | Proc slack | CPU% and IO% vs extra worker count |
| `throughput_vs_utilization.png` | Proc slack | Normalized baseline throughput % of peak vs utilization % |
| `per_worker_distribution.png` | Proc slack | Side-by-side box plots: baseline workers (blue) vs extra workers (orange) |
| `per_worker_fairness.png` | Proc slack | Baseline total throughput vs fairness CV% |

## Configuration

The `compose.yaml` includes isolation settings:
- `cpuset`: Pin to specific CPU cores (adjust to match your system)
- `mem_limit` / `mem_reservation`: Fixed memory allocation (8GB limit, 4GB reserved)
- `blkio_config`: I/O throttle — 100 MB/s bandwidth and 10,000 IOPS read/write. Device path is hardware-specific — adjust for your system.
- `volumes`: `./io_scratch:/tmp` for I/O operations
- `network_mode: none`: No network interference
- `privileged: true`: Required for shared memory operations
- `ulimits`: Raised file descriptor limits for high process counts
