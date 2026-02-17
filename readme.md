# Saturator

A tool for measuring CPU and I/O saturation points and finding slack capacity in containerized environments. Supports both thread-based and process-based experiments with configurable workload parameters.

## Purpose

Saturator helps answer:
1. How many CPU-bound threads can run before throughput plateaus?
2. How many I/O-bound threads can run before throughput plateaus?
3. How many additional threads (at a given I/O ratio) can be added to a saturated baseline without degrading performance?
4. At what point does adding more worker **processes** cause throughput to actually **drop** (not just plateau)?

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

**Output:** `proc_cpu_throughput_vs_workers.csv`

#### Find I/O Saturation (Processes)
```bash
docker compose run --build --remove-orphans saturator find-io-saturation-proc
```

**Output:** `proc_io_throughput_vs_workers.csv`

### Tuning Options

The `-proc` experiments accept optional flags to control workload parameters. These can also be used with the thread-based experiments.

| Flag | Default | Description |
|------|---------|-------------|
| `--target-us <N>` | 50 | Calibration target per-operation in microseconds. Lower values (e.g. 1-5) make context switch overhead proportionally larger. |
| `--buffer-kb <N>` | 100 | CPU work buffer size in KB. Larger values (e.g. 1024-10240) cause L3 cache contention at high worker counts. |
| `--io-buffer-kb <N>` | 4 | IO read/write buffer size in KB. Larger values (e.g. 64-1024) increase bytes per IO operation, driving higher bandwidth utilization. |
| `--max-workers <N>` | parallelism * 16 | Maximum number of workers to test. |
| `--duration <N>` | 5 | Measurement duration per data point in seconds. |
| `--samples <N>` | 3 | Samples per data point. Median and stddev are computed; stddev is written to CSV for error bar visualization. |
| `--step <N>` | 1 | Worker count increment per data point. |
| `--intensity <F>` | 1.0 | Work probability per iteration (0.0–1.0). Each idle iteration sleeps for `target_us` μs instead of working. Simulates partially-loaded workers. |

Examples:
```bash
# Small work units + large buffers to maximize degradation visibility
docker compose run --build --remove-orphans saturator find-saturation-proc \
    --target-us 5 --buffer-kb 1024 --max-workers 100

# Compare threads vs processes with same parameters
docker compose run --build --remove-orphans saturator find-saturation \
    --max-workers 64
docker compose run --build --remove-orphans saturator find-saturation-proc \
    --max-workers 64
```

## Output

Results are written to CSV files in the `./output` directory.

### Saturation CSV columns

Produced by `find-saturation`, `find-io-saturation`, `find-saturation-proc`, and `find-io-saturation-proc`.

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
| `io_util_pct` | IO bandwidth utilization as percentage of cgroup limit (from `io.stat` / `io.max`) |
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

### Plotting

```bash
python plot_results.py              # Plot all CSVs in ./output
python plot_results.py output/*.csv # Plot specific files
```

Generates PNG visualizations with stddev error bars:
- Throughput vs thread/worker count (saturation curves)
- Baseline degradation vs extra threads (slack analysis)

## Configuration

The `compose.yaml` includes isolation settings:
- `cpuset`: Pin to specific CPU cores (adjust to match your system)
- `mem_limit` / `mem_reservation`: Fixed memory allocation (8GB limit, 4GB reserved)
- `blkio_config`: I/O throughput throttle (100 MB/s read/write) to simulate spinning disk speeds. Device path is hardware-specific — adjust for your system.
- `volumes`: `./io_scratch:/tmp` for I/O operations
- `network_mode: none`: No network interference
- `privileged: true`: Required for shared memory operations
- `ulimits`: Raised file descriptor limits for high process counts
