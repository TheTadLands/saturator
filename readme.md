# Saturator

A tool for measuring CPU and I/O saturation points and finding slack capacity in containerized environments.

## Purpose

Saturator helps answer:
1. How many CPU-bound threads can run before throughput plateaus?
2. How many I/O-bound threads can run before throughput plateaus?
3. How many additional threads (at a given I/O ratio) can be added to a saturated baseline without degrading performance?

## Running

```bash
docker compose run --build --remove-orphans saturator <experiment> [args]
```

- `--build` ensures you're running the latest code
- `--remove-orphans` cleans up old containers

## Experiments

### Find CPU Saturation
Incrementally adds CPU-only threads until throughput plateaus.

```bash
docker compose run --build --remove-orphans saturator find-saturation
```

**Output:** `cpu_saturation.csv`

### Find I/O Saturation
Incrementally adds I/O-only threads until throughput plateaus.

```bash
docker compose run --build --remove-orphans saturator find-io-saturation
```

**Output:** `io_saturation.csv`

### Find Slack (CPU Baseline)
Starts with N CPU-only baseline threads, then adds threads at a specified I/O percentage until baseline degrades.

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

**Output:** `cpu_slack_<N>t_extra<io%>io.csv`

### Find I/O Slack (I/O Baseline)
Starts with N I/O-only baseline threads, then adds threads at a specified I/O percentage until baseline degrades.

```bash
docker compose run --build --remove-orphans saturator find-io-slack <N> <extra_io%>
```

Arguments:
- `N` - Number of I/O-only baseline threads
- `extra_io%` - I/O percentage for extra threads being added

Examples:
```bash
# 4 I/O-only baseline threads, add CPU-only threads
docker compose run --build --remove-orphans saturator find-io-slack 4 0

# 4 I/O-only baseline threads, add 50% I/O threads
docker compose run --build --remove-orphans saturator find-io-slack 4 50

# 4 I/O-only baseline threads, add I/O-only threads
docker compose run --build --remove-orphans saturator find-io-slack 4 100
```

**Output:** `io_slack_<N>t_extra<io%>io.csv`

## Output

Results are written to CSV files in the `./output` directory. These can be plotted to visualize:
- Throughput vs thread count (saturation curves)
- Baseline degradation vs extra threads (slack analysis)

## Configuration

The `compose.yaml` includes isolation settings:
- `cpuset`: Pin to specific CPU cores
- `mem_limit`: Fixed memory allocation
- `tmpfs` on `/tmp`: RAM-based I/O for consistent benchmarks
- `network_mode: none`: No network interference

Adjust `cpuset` in `compose.yaml` to match your system's available cores.
