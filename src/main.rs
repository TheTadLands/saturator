mod saturator;
mod visualize;
mod proc_metrics;

use saturator::{
    calibrate_operations_full, CalibrationResult, TuningParams,
    run_saturator, run_worker_process,
    create_shared_region, destroy_shared_region, worker_counters,
};
use visualize::ResultsWriter;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: saturator <experiment> [args] [OPTIONS]");
        println!("Experiments:");
        println!("  find-saturation                - Find CPU saturation point (threads)");
        println!("  find-io-saturation             - Find I/O saturation point (threads)");
        println!("  find-slack <N> <extra_io%>     - N CPU baseline threads, add threads at extra_io%");
        println!("  find-io-slack <N> <extra_io%>  - N I/O baseline threads, add threads at extra_io%");
        println!("  find-saturation-proc           - Find CPU saturation point (processes)");
        println!("  find-io-saturation-proc        - Find I/O saturation point (processes)");
        println!("  find-mixed-saturation-proc <io_pct> - Mixed CPU+IO processes at given IO% (0-100)");
        println!("  find-saturation-intensity-proc <N> <io_pct> - N base procs + 1 probe, sweep probe intensity");
        println!("  find-slack-proc <N> <extra_io%>    - N CPU baseline procs, add procs at extra_io%");
        println!("  find-io-slack-proc <N> <extra_io%> - N I/O baseline procs, add procs at extra_io%");
        println!("");
        println!("Options (for -proc variants):");
        println!("  --buffer-kb <N>      CPU work buffer size in KB (default: 100)");
        println!("  --io-buffer-kb <N>   IO read/write buffer size in KB (default: 4)");
        println!("  --max-workers <N>    Max worker count (default: parallelism * 16)");
        println!("  --duration <N>       Measurement duration in seconds (default: 5)");
        println!("  --samples <N>        Samples per data point, median taken (default: 5)");
        println!("  --step <N>           Worker count increment per data point (default: 1)");
        println!("  --intensity <F>      Work probability per iteration, 0.0-1.0 (default: 1.0)");
        println!("  --chain              Auto-run intensity sweep at saturation point (proc only)");
        println!("  --warmup <N>         Warmup duration in seconds before measurement (default: 1)");
        println!("");
        println!("Examples:");
        println!("  find-slack 4 100     - 4 CPU baseline, add 100% I/O threads");
        println!("  find-saturation-proc --buffer-kb 1024 --max-workers 100 --samples 7");
        println!("  find-mixed-saturation-proc 50 --max-workers 32");
        println!("  find-saturation-intensity-proc 6 50 --duration 2");
        return;
    }

    let experiment = &args[1];

    // Hidden worker subcommand — child process entry point
    if experiment == "__worker" {
        // Args: __worker <shm_name> <worker_id> <cpu_iters> <io_iters> <io_perc> <buffer_kb> <io_buffer_kb> <intensity> <sleep_us> <max_workers>
        let shm_name = &args[2];
        let worker_id: usize = args[3].parse().unwrap();
        let cpu_iters: usize = args[4].parse().unwrap();
        let io_iters: usize = args[5].parse().unwrap();
        let io_perc: f64 = args[6].parse().unwrap();
        let buffer_kb: usize = args[7].parse().unwrap();
        let io_buffer_kb: usize = args[8].parse().unwrap();
        let intensity: f64 = args[9].parse().unwrap();
        let sleep_us: u64 = args[10].parse().unwrap();
        let max_workers: usize = args[11].parse().unwrap();

        run_worker_process(shm_name, worker_id, cpu_iters, io_iters, io_perc, buffer_kb * 1024, io_buffer_kb * 1024, intensity, sleep_us, max_workers);
        return;
    }

    // Clean up any leftover files from previous runs
    cleanup_scratch_files();

    // Parse tuning parameters from optional flags
    let params = parse_tuning_params(&args);

    println!("Calibrating operations...");
    let calibration = calibrate_operations_full(&params);
    println!("Calibration: {} CPU iterations/op ({}μs), {} I/O iterations/op ({}μs)",
             calibration.cpu_iterations, calibration.cpu_us,
             calibration.io_iterations, calibration.io_us);
    println!("Theoretical max: {:.0} CPU work-units/s, {:.0} I/O work-units/s per thread\n",
             calibration.cpu_ops_per_sec(), calibration.io_ops_per_sec());

    match experiment.as_str() {
        "find-saturation" => { run_saturation_experiment(SaturationExperiment {
            label: "CPU",
            mode: Mode::Threads,
            io_perc: 0.0,
            csv_base: "cpu_throughput_vs_threads",
            recommendation: Some("find-io-slack"),
        }, calibration, &params); },
        "find-io-saturation" => { run_saturation_experiment(SaturationExperiment {
            label: "I/O",
            mode: Mode::Threads,
            io_perc: 1.0,
            csv_base: "io_throughput_vs_threads",
            recommendation: Some("find-cpu-slack"),
        }, calibration, &params); },
        "find-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(100.0);
            run_slack_experiment(SlackExperiment {
                baseline_label: "CPU",
                baseline_io_ratio: 0.0,
                tracked_label: "CPU",
                track_io: false,
            }, calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-io-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            run_slack_experiment(SlackExperiment {
                baseline_label: "I/O",
                baseline_io_ratio: 1.0,
                tracked_label: "I/O",
                track_io: true,
            }, calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-saturation-proc" => {
            let result = run_saturation_experiment(SaturationExperiment {
                label: "CPU",
                mode: Mode::Procs,
                io_perc: 0.0,
                csv_base: "proc_cpu_throughput_vs_workers",
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, 0, calibration, &params);
                }
            }
        },
        "find-io-saturation-proc" => {
            let result = run_saturation_experiment(SaturationExperiment {
                label: "I/O",
                mode: Mode::Procs,
                io_perc: 1.0,
                csv_base: "proc_io_throughput_vs_workers",
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, 100, calibration, &params);
                }
            }
        },
        "find-mixed-saturation-proc" => {
            let io_pct = args.get(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-mixed-saturation-proc <io_pct> [OPTIONS]");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_perc = (io_pct.clamp(0.0, 100.0)) / 100.0;
            let io_pct_int = io_pct as u32;
            let csv_base = format!("proc_mixed_{}pct_io_throughput_vs_workers", io_pct_int);
            let result = run_saturation_experiment(SaturationExperiment {
                label: "Mixed",
                mode: Mode::Procs,
                io_perc,
                csv_base: Box::leak(csv_base.into_boxed_str()),
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, io_pct_int, calibration, &params);
                }
            }
        },
        "find-saturation-intensity-proc" => {
            let base_workers = args.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-saturation-intensity-proc <N> <io_pct> [OPTIONS]");
                eprintln!("  N: number of base workers at intensity=1.0");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_pct_int = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-saturation-intensity-proc <N> <io_pct> [OPTIONS]");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_perc = (io_pct_int as f64).clamp(0.0, 100.0) / 100.0;
            run_intensity_sweep_experiment(base_workers, io_perc, io_pct_int, calibration, &params);
        },
        "find-slack-proc" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            run_slack_proc_experiment("CPU", 0.0, false, baseline, extra_io_pct / 100.0, calibration, &params);
        },
        "find-io-slack-proc" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-io-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-io-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            run_slack_proc_experiment("I/O", 1.0, true, baseline, extra_io_pct / 100.0, calibration, &params);
        },
        _ => println!("Unknown experiment: {}", experiment),
    }

    // Clean up after run
    cleanup_scratch_files();
}

fn parse_tuning_params(args: &[String]) -> TuningParams {
    let mut params = TuningParams::default();
    // For -proc variants, use higher default max_workers
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get()).unwrap_or(4);
    if args.len() >= 2 && args[1].ends_with("-proc") {
        params.max_workers = parallelism * 16;
    }

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--buffer-kb" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.buffer_kb = v;
                }
            }
            "--io-buffer-kb" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.io_buffer_kb = v;
                }
            }
            "--max-workers" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.max_workers = v;
                }
            }
            "--duration" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.duration_secs = v;
                }
            }
            "--samples" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    params.samples = v.max(1);
                }
            }
            "--step" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    params.step = v.max(1);
                }
            }
            "--intensity" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<f64>().ok()) {
                    params.intensity = v.clamp(0.0, 1.0);
                }
            }
            "--chain" => {
                params.chain = true;
            }
            "--warmup" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.warmup_secs = v;
                }
            }
            _ => {}
        }
        i += 1;
    }
    params
}

fn cleanup_scratch_files() {
    // Clean up saturator temp files
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("saturator") {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

fn timestamp() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Convert to YYYYMMDD_HHMMSS in UTC
    let s = secs;
    let days = s / 86400;
    let time = s % 86400;
    let hours = time / 3600;
    let minutes = (time % 3600) / 60;
    let seconds = time % 60;
    // Days since epoch to Y/M/D
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for &md in &month_days {
        if remaining < md as i64 { break; }
        remaining -= md as i64;
        m += 1;
    }
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, m + 1, remaining + 1, hours, minutes, seconds)
}

fn write_params_file(dir: &str, experiment: &str, params: &TuningParams, calibration: &CalibrationResult, extra: &[(&str, String)]) {
    use std::io::Write;
    let path = format!("{}/params.txt", dir);
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "experiment: {}", experiment).unwrap();
    writeln!(f, "buffer_kb: {}", params.buffer_kb).unwrap();
    writeln!(f, "io_buffer_kb: {}", params.io_buffer_kb).unwrap();
    writeln!(f, "max_workers: {}", params.max_workers).unwrap();
    writeln!(f, "duration_secs: {}", params.duration_secs).unwrap();
    writeln!(f, "samples: {}", params.samples).unwrap();
    writeln!(f, "step: {}", params.step).unwrap();
    writeln!(f, "intensity: {}", params.intensity).unwrap();
    writeln!(f, "warmup_secs: {}", params.warmup_secs).unwrap();
    writeln!(f, "cpu_iterations: {}", calibration.cpu_iterations).unwrap();
    writeln!(f, "io_iterations: {}", calibration.io_iterations).unwrap();
    writeln!(f, "cpu_us: {}", calibration.cpu_us).unwrap();
    writeln!(f, "io_us: {}", calibration.io_us).unwrap();
    for (key, val) in extra {
        writeln!(f, "{}: {}", key, val).unwrap();
    }
}

// === Unified saturation experiments ===

#[derive(Clone, Copy)]
enum Mode {
    Threads,
    Procs,
}

struct SaturationExperiment {
    label: &'static str,
    mode: Mode,
    io_perc: f64,
    csv_base: &'static str,
    recommendation: Option<&'static str>,
}

fn run_saturation_experiment(
    exp: SaturationExperiment,
    calibration: CalibrationResult,
    params: &TuningParams,
) -> Option<(usize, f64)> {
    let (mode_label, worker_label) = match exp.mode {
        Mode::Threads => ("", "threads"),
        Mode::Procs => (" (PROCESS MODE)", "procs"),
    };

    println!("=== FINDING {} SATURATION POINT{} ===", exp.label, mode_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --max-workers={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb, params.max_workers,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 {
        print!(", --intensity={:.2}", params.intensity);
    }
    if params.chain {
        print!(", --chain");
    }
    println!();
    println!("Adding {} {} until throughput {}\n",
             exp.label, worker_label,
             if matches!(exp.mode, Mode::Threads) { "plateaus" } else { "degrades" });

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = params.step;
    let mut per_worker_rows: Vec<(usize, Vec<(f64, f64, f64)>)> = Vec::new();

    let use_step = matches!(exp.mode, Mode::Procs);
    let mut worker_count = if use_step { params.step } else { 1 };
    let end = params.max_workers;

    let wlabel = if use_step { "procs" } else { "threads" };
    println!("  {:>7} | {:>12} {:>12} {:>12} | {:>10} | {:>6} {:>6}",
             wlabel, "cpu ops/s", "io ops/s", "total ops/s", "per worker", "cpu%", "io%");
    println!("  {}", "-".repeat(82));

    while worker_count <= end {
        let sleep_us = calibration.cpu_us as u64;
        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_worker_data) = match exp.mode {
            Mode::Threads => {
                let (c, i, cs, is, m, pw) = measure_thread_throughput(
                    worker_count, calibration.cpu_iterations, calibration.io_iterations,
                    params.duration_secs, exp.io_perc, params.buffer_kb * 1024, params.io_buffer_kb * 1024, params.samples,
                    params.intensity, sleep_us, params.warmup_secs,
                );
                (c, i, cs, is, m, Some(pw))
            },
            Mode::Procs => {
                let (c, i, cs, is, m, pw) = measure_proc_throughput(
                    worker_count, calibration.cpu_iterations, calibration.io_iterations,
                    params.duration_secs, exp.io_perc, params.buffer_kb, params.io_buffer_kb, params.samples,
                    params.intensity, sleep_us, params.warmup_secs,
                );
                (c, i, cs, is, m, Some(pw))
            },
        };
        let total_ops = cpu_ops + io_ops;
        let throughput_per_worker = total_ops / worker_count as f64;

        println!("  {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>10.0} | {:>5.1}% {:>5.1}%",
                 worker_count, cpu_ops, io_ops, total_ops, throughput_per_worker,
                 metrics.cpu_pct, metrics.io_util_pct);

        writer.add_saturation_point(worker_count, cpu_ops, io_ops, cpu_stddev, io_stddev, metrics);

        if let Some(pw) = per_worker_data {
            per_worker_rows.push((worker_count, pw));
        }

        if total_ops > best_throughput {
            best_throughput = total_ops;
            saturation_point = worker_count;
        }

        if use_step {
            worker_count += params.step;
        } else {
            worker_count += 1;
        }
    }

    let run_dir = format!("{}_{}", exp.csv_base, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    let mode_str = match exp.mode { Mode::Threads => "threads", Mode::Procs => "procs" };
    write_params_file(&run_dir, &format!("{} saturation ({})", exp.label, mode_str), params, &calibration, &[
        ("io_perc", format!("{}", exp.io_perc)),
        ("mode", mode_str.to_string()),
    ]);

    let csv_path = format!("{}/{}.csv", run_dir, exp.csv_base);
    writer.write_saturation_csv(std::path::Path::new(&csv_path)).unwrap();

    // Write per-worker CSV (both thread and proc mode)
    if !per_worker_rows.is_empty() {
        use std::io::Write as _;
        let pw_path = format!("{}/per_worker_{}.csv", run_dir, exp.csv_base);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (workers, per_worker) in &per_worker_rows {
            for (wid, (wc, wi, ws)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{},{},{:.2},{:.2},{:.2},{:.2}", workers, wid, wc, wi, ws, wc + wi).unwrap();
            }
        }
    }

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} {}", saturation_point, worker_label);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    if let Some(rec) = exp.recommendation {
        println!("\nRecommendation: Run `saturator {} {}` to find {} slack",
                 rec, saturation_point,
                 if exp.io_perc < 0.5 { "I/O" } else { "CPU" });
    }

    match exp.mode {
        Mode::Procs => Some((saturation_point, exp.io_perc)),
        Mode::Threads => None,
    }
}

// === Unified slack experiments ===

struct SlackExperiment {
    baseline_label: &'static str,
    baseline_io_ratio: f64,
    tracked_label: &'static str,
    track_io: bool,
}

fn run_slack_experiment(
    exp: SlackExperiment,
    calibration: CalibrationResult,
    params: &TuningParams,
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK ({} BASELINE) ===", exp.baseline_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 { print!(", --intensity={:.2}", params.intensity); }
    println!();
    println!("Baseline: {} {}-only threads", baseline_threads, exp.baseline_label);
    println!("Adding threads at: {:.0}% I/O\n", extra_io_ratio * 100.0);

    let buffer_size = params.buffer_kb * 1024;

    println!("Measuring baseline {} throughput...", exp.baseline_label);
    let sleep_us = calibration.cpu_us as u64;
    let io_buffer_size = params.io_buffer_kb * 1024;
    let (baseline_cpu, baseline_io, baseline_cpu_std, baseline_io_std, baseline_metrics, baseline_per_thread) = measure_baseline(
        baseline_threads, exp.baseline_io_ratio,
        calibration.cpu_iterations, calibration.io_iterations,
        params.duration_secs, buffer_size, io_buffer_size, params.samples,
        params.intensity, sleep_us, params.warmup_secs,
    );
    let baseline_throughput = if exp.track_io { baseline_io } else { baseline_cpu };
    println!("Baseline: {:.0} {} ops/sec (cpu: {:.0}, io: {:.0})\n",
             baseline_throughput, exp.baseline_label, baseline_cpu, baseline_io);

    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_extra = logical_cores.max(baseline_threads * 2);
    let mut best_tracked_ops = baseline_throughput;
    let mut best_extra_count = 0;

    let csv_base = format!("slack_{}{}_adding_{}pct_io",
                           baseline_threads,
                           exp.baseline_label.to_lowercase(),
                           (extra_io_ratio * 100.0) as u32);
    let run_dir = format!("{}_{}", csv_base, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("{} slack", exp.baseline_label), params, &calibration, &[
        ("baseline_threads", format!("{}", baseline_threads)),
        ("baseline_io_ratio", format!("{}", exp.baseline_io_ratio)),
        ("extra_io_ratio", format!("{}", extra_io_ratio)),
    ]);

    let filename = format!("{}/{}.csv", run_dir, csv_base);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,baseline_change_pct,cpu_ops_stddev,io_ops_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    // Write extra=0 baseline row
    writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, baseline_threads,
             (extra_io_ratio * 100.0) as u32,
             baseline_cpu, baseline_io, baseline_cpu + baseline_io, 0.0,
             baseline_cpu_std, baseline_io_std,
             baseline_metrics.to_csv_row()).unwrap();

    let vs_label = if exp.track_io { "vs base" } else { "vs base" };
    println!("  {:>5} {:>7} | {:>12} {:>12} {:>12} | {:>8} | {:>6} {:>6}",
             "extra", "total", "cpu ops/s", "io ops/s", "total ops/s", vs_label, "cpu%", "io%");
    println!("  {}", "-".repeat(90));

    // Per-thread CSV: seed with extra=0 baseline data
    let pw_filename = format!("{}/per_worker_{}.csv", run_dir, csv_base);
    let mut pw_file = {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&pw_filename).unwrap();
        writeln!(f, "extra_threads,total_threads,thread_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (tid, (wc, wi, ws)) in baseline_per_thread.iter().enumerate() {
            writeln!(f, "{},{},{},{:.2},{:.2},{:.2},{:.2}", 0, baseline_threads, tid, wc, wi, ws, wc + wi).unwrap();
        }
        f
    };

    for extra in 1..=max_extra {
        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_thread) = measure_total_throughput(
            baseline_threads, extra, exp.baseline_io_ratio, extra_io_ratio,
            calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, io_buffer_size, params.samples,
            params.intensity, sleep_us, params.warmup_secs,
        );

        let total_ops = cpu_ops + io_ops;
        let cpu_ops_stddev = cpu_stddev;
        let io_ops_stddev = io_stddev;
        let tracked_ops = if exp.track_io { io_ops } else { cpu_ops };
        let baseline_change = (tracked_ops - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>5} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>+7.1}% | {:>5.1}% {:>5.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, baseline_change,
                 metrics.cpu_pct, metrics.io_util_pct);

        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, baseline_change,
                 cpu_ops_stddev, io_ops_stddev,
                 metrics.to_csv_row()).unwrap();

        use std::io::Write as _;
        for (tid, (wc, wi, ws)) in per_thread.iter().enumerate() {
            writeln!(pw_file, "{},{},{},{:.2},{:.2},{:.2},{:.2}",
                     extra, baseline_threads + extra, tid, wc, wi, ws, wc + wi).unwrap();
        }

        if tracked_ops > best_tracked_ops {
            best_tracked_ops = tracked_ops;
            best_extra_count = extra;
        }
    }

    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} {} ops/sec",
             baseline_threads, baseline_throughput, exp.baseline_label);
    println!("Best {}: {} threads = {:.0} {} ops/sec ({:+.1}%)",
             exp.tracked_label,
             baseline_threads + best_extra_count, best_tracked_ops, exp.tracked_label,
             (best_tracked_ops - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}

// === Measurement helpers ===

fn measure_thread_throughput(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_size: usize,
    io_buffer_size: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<(f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>)> = (0..num_samples).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_size, io_buffer_size, intensity, sleep_us, warmup_secs)
    }).collect();
    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();

    // Pick per-thread data from the sample closest to median total throughput
    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_thread = samples[median_idx].3.clone();

    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list), per_thread)
}

fn measure_proc_throughput(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<(f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>)> = (0..num_samples).map(|_| {
        measure_single_run_proc(worker_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_kb, io_buffer_kb, intensity, sleep_us, warmup_secs)
    }).collect();
    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();

    // Pick per-worker data from the sample closest to median total throughput
    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].3.clone();

    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list), per_worker)
}

fn measure_single_run_proc(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let shm_name = format!("/saturator_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, worker_count);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();

    for i in 0..worker_count {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(worker_count.to_string())
            .spawn()
            .expect("Failed to spawn worker process");
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(10));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    // Reset per-worker counters after warmup
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);

    // Reap children so they flush remaining local_ops
    for mut child in children {
        let _ = child.wait();
    }

    let cpu = region.cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
    let io = region.io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    // Read per-worker counters
    let mut per_worker = Vec::with_capacity(worker_count);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        per_worker.push((wc, wi, ws));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, worker_count);

    (cpu, io, metrics, per_worker)
}

fn measure_baseline(
    threads: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    io_buffer_size: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<(f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>)> = (0..num_samples).map(|_| {
        let cpu_ops = Arc::new(AtomicU64::new(0));
        let io_ops = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));

        let per_thread: Vec<[Arc<AtomicU64>; 3]> = (0..threads)
            .map(|_| [Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0))])
            .collect();

        let mut handles = vec![];

        for (i, pt) in per_thread.iter().enumerate() {
            let running = Arc::clone(&running);
            let cpu_ops = Arc::clone(&cpu_ops);
            let io_ops = Arc::clone(&io_ops);
            let pt_cpu   = Arc::clone(&pt[0]);
            let pt_io    = Arc::clone(&pt[1]);
            let pt_sleep = Arc::clone(&pt[2]);

            let handle = std::thread::spawn(move || {
                run_saturator(i, io_ratio, cpu_iterations, io_iterations, buffer_size, io_buffer_size,
                              cpu_ops, io_ops, running, intensity, sleep_us, pt_cpu, pt_io, pt_sleep);
            });
            handles.push(handle);
        }

        std::thread::sleep(Duration::from_secs(warmup_secs));
        cpu_ops.store(0, Ordering::Relaxed);
        io_ops.store(0, Ordering::Relaxed);
        for pt in &per_thread {
            pt[0].store(0, Ordering::Relaxed);
            pt[1].store(0, Ordering::Relaxed);
            pt[2].store(0, Ordering::Relaxed);
        }

        let snap_before = proc_metrics::take_snapshot();
        std::thread::sleep(Duration::from_secs(duration_secs));
        let snap_after = proc_metrics::take_snapshot();
        let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

        running.store(false, Ordering::Relaxed);
        for handle in handles {
            handle.join().unwrap();
        }

        let cpu = cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let io = io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let pt_data: Vec<(f64, f64, f64)> = per_thread.iter()
            .map(|pt| (
                pt[0].load(Ordering::Relaxed) as f64 / duration_secs as f64,
                pt[1].load(Ordering::Relaxed) as f64 / duration_secs as f64,
                pt[2].load(Ordering::Relaxed) as f64 / duration_secs as f64,
            ))
            .collect();
        (cpu, io, metrics, pt_data)
    }).collect();

    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();

    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            ((a.0 + a.1) - median_total).abs()
                .partial_cmp(&((b.0 + b.1) - median_total).abs()).unwrap()
        })
        .map(|(i, _)| i).unwrap_or(0);
    let per_thread = samples[median_idx].3.clone();

    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list), per_thread)
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn stddev(samples: &[f64]) -> f64 {
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    variance.sqrt()
}

fn measure_single_run(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_size: usize,
    io_buffer_size: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let cpu_ops = Arc::new(AtomicU64::new(0));
    let io_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));

    // Per-thread counters: [cpu, io, sleep] per thread
    let per_thread: Vec<[Arc<AtomicU64>; 3]> = (0..thread_count)
        .map(|_| [Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0))])
        .collect();

    let mut handles = vec![];

    for (i, pt) in per_thread.iter().enumerate() {
        let running = Arc::clone(&running);
        let cpu_ops = Arc::clone(&cpu_ops);
        let io_ops = Arc::clone(&io_ops);
        let pt_cpu   = Arc::clone(&pt[0]);
        let pt_io    = Arc::clone(&pt[1]);
        let pt_sleep = Arc::clone(&pt[2]);

        let handle = std::thread::spawn(move || {
            run_saturator(i, io_perc, cpu_iterations, io_iterations, buffer_size, io_buffer_size,
                          cpu_ops, io_ops, running, intensity, sleep_us, pt_cpu, pt_io, pt_sleep);
        });
        handles.push(handle);
    }

    // Warmup: let threads run, then reset counters
    std::thread::sleep(Duration::from_secs(warmup_secs));
    cpu_ops.store(0, Ordering::Relaxed);
    io_ops.store(0, Ordering::Relaxed);
    for pt in &per_thread {
        pt[0].store(0, Ordering::Relaxed);
        pt[1].store(0, Ordering::Relaxed);
        pt[2].store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    // Now measure for the actual duration
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    let cpu = cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
    let io = io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    let per_thread_data: Vec<(f64, f64, f64)> = per_thread.iter()
        .map(|pt| (
            pt[0].load(Ordering::Relaxed) as f64 / duration_secs as f64,
            pt[1].load(Ordering::Relaxed) as f64 / duration_secs as f64,
            pt[2].load(Ordering::Relaxed) as f64 / duration_secs as f64,
        ))
        .collect();

    (cpu, io, metrics, per_thread_data)
}

fn measure_total_throughput(
    baseline_threads: usize,
    extra_threads: usize,
    baseline_io_ratio: f64,
    extra_io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    io_buffer_size: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let results: Vec<(f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>)> = (0..num_samples).map(|_| {
        measure_total_throughput_single(
            baseline_threads, extra_threads, baseline_io_ratio, extra_io_ratio,
            cpu_iterations, io_iterations, duration_secs, buffer_size, io_buffer_size,
            intensity, sleep_us, warmup_secs,
        )
    }).collect();

    let cpu_vals: Vec<f64> = results.iter().map(|r| r.0).collect();
    let io_vals: Vec<f64> = results.iter().map(|r| r.1).collect();

    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = results.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            ((a.0 + a.1) - median_total).abs()
                .partial_cmp(&((b.0 + b.1) - median_total).abs()).unwrap()
        })
        .map(|(i, _)| i).unwrap_or(0);
    let per_thread = results[median_idx].3.clone();

    let metrics_list: Vec<proc_metrics::SystemMetrics> = results.into_iter().map(|r| r.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list), per_thread)
}

fn measure_total_throughput_single(
    baseline_threads: usize,
    extra_threads: usize,
    baseline_io_ratio: f64,
    extra_io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    io_buffer_size: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let total_threads = baseline_threads + extra_threads;
    let cpu_ops = Arc::new(AtomicU64::new(0));
    let io_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));

    let per_thread: Vec<[Arc<AtomicU64>; 3]> = (0..total_threads)
        .map(|_| [Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0))])
        .collect();

    let mut handles = vec![];

    // Spawn baseline threads
    for (i, pt) in per_thread[..baseline_threads].iter().enumerate() {
        let running = Arc::clone(&running);
        let cpu_ops = Arc::clone(&cpu_ops);
        let io_ops = Arc::clone(&io_ops);
        let io_ratio = baseline_io_ratio;
        let pt_cpu   = Arc::clone(&pt[0]);
        let pt_io    = Arc::clone(&pt[1]);
        let pt_sleep = Arc::clone(&pt[2]);

        let handle = std::thread::spawn(move || {
            run_saturator_split(i, io_ratio, cpu_iterations, io_iterations, buffer_size, io_buffer_size,
                                cpu_ops, io_ops, running, intensity, sleep_us, pt_cpu, pt_io, pt_sleep);
        });
        handles.push(handle);
    }

    // Spawn extra threads
    for (i, pt) in per_thread[baseline_threads..].iter().enumerate() {
        let running = Arc::clone(&running);
        let cpu_ops = Arc::clone(&cpu_ops);
        let io_ops = Arc::clone(&io_ops);
        let io_ratio = extra_io_ratio;
        let pt_cpu   = Arc::clone(&pt[0]);
        let pt_io    = Arc::clone(&pt[1]);
        let pt_sleep = Arc::clone(&pt[2]);

        let handle = std::thread::spawn(move || {
            run_saturator_split(baseline_threads + i, io_ratio, cpu_iterations, io_iterations, buffer_size, io_buffer_size,
                                cpu_ops, io_ops, running, intensity, sleep_us, pt_cpu, pt_io, pt_sleep);
        });
        handles.push(handle);
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(warmup_secs));
    cpu_ops.store(0, Ordering::Relaxed);
    io_ops.store(0, Ordering::Relaxed);
    for pt in &per_thread {
        pt[0].store(0, Ordering::Relaxed);
        pt[1].store(0, Ordering::Relaxed);
        pt[2].store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    let cpu = cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
    let io = io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    let per_thread_data: Vec<(f64, f64, f64)> = per_thread.iter()
        .map(|pt| (
            pt[0].load(Ordering::Relaxed) as f64 / duration_secs as f64,
            pt[1].load(Ordering::Relaxed) as f64 / duration_secs as f64,
            pt[2].load(Ordering::Relaxed) as f64 / duration_secs as f64,
        ))
        .collect();

    (cpu, io, metrics, per_thread_data)
}

// === Intensity sweep experiment ===

fn run_intensity_sweep_experiment(
    base_workers: usize,
    io_perc: f64,
    io_pct_int: u32,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    println!("=== INTENSITY SWEEP (PROCESS MODE) ===");
    println!("{} base workers at intensity=1.0, io_pct={}%", base_workers, io_pct_int);
    println!("Sweeping 1 probe worker intensity from 0.0 to 1.0\n");

    let sleep_us = calibration.cpu_us as u64;

    let run_dir = format!("proc_intensity_sweep_{}base_{}pct_io_{}", base_workers, io_pct_int, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, "intensity sweep", params, &calibration, &[
        ("base_workers", format!("{}", base_workers)),
        ("io_pct", format!("{}", io_pct_int)),
        ("io_perc", format!("{}", io_perc)),
    ]);

    let csv_path = format!("{}/proc_intensity_sweep_{}base_{}pct_io.csv", run_dir, base_workers, io_pct_int);
    let mut file = std::fs::File::create(&csv_path).unwrap();
    use std::io::Write as _;
    writeln!(file, "probe_intensity,workers,cpu_ops_sec,io_ops_sec,total_ops_sec,cpu_ops_stddev,io_ops_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    println!("  {:>9} | {:>12} {:>12} {:>12} | {:>6} {:>6}",
             "intensity", "cpu ops/s", "io ops/s", "total ops/s", "cpu%", "io%");
    println!("  {}", "-".repeat(72));

    let mut best_throughput = 0.0;
    let mut best_intensity = 0.0;
    let total_workers = base_workers + 1;
    let mut per_worker_rows: Vec<(f64, Vec<(f64, f64, f64)>)> = Vec::new();

    for step in 0..=20 {
        let probe_intensity = step as f64 * 0.05;

        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_worker) = measure_proc_throughput_mixed_intensity(
            base_workers, probe_intensity, calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, io_perc, params.buffer_kb, params.io_buffer_kb, params.samples,
            sleep_us, params.warmup_secs,
        );
        let total_ops = cpu_ops + io_ops;

        println!("  {:>9.2} | {:>12.0} {:>12.0} {:>12.0} | {:>5.1}% {:>5.1}%",
                 probe_intensity, cpu_ops, io_ops, total_ops,
                 metrics.cpu_pct, metrics.io_util_pct);

        writeln!(file, "{:.2},{},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 probe_intensity, total_workers, cpu_ops, io_ops, total_ops,
                 cpu_stddev, io_stddev, metrics.to_csv_row()).unwrap();

        per_worker_rows.push((probe_intensity, per_worker));

        if total_ops > best_throughput {
            best_throughput = total_ops;
            best_intensity = probe_intensity;
        }
    }

    // Write per-worker CSV
    {
        let pw_path = format!("{}/per_worker_proc_intensity_sweep_{}base_{}pct_io.csv", run_dir, base_workers, io_pct_int);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "probe_intensity,workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (intensity, per_worker) in &per_worker_rows {
            for (wid, (wc, wi, ws)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{:.2},{},{},{:.2},{:.2},{:.2},{:.2}", intensity, total_workers, wid, wc, wi, ws, wc + wi).unwrap();
            }
        }
    }

    println!("\n=== RESULTS ===");
    println!("Best throughput: {:.0} ops/sec at probe intensity {:.2}", best_throughput, best_intensity);
    println!("Results written to: {}", csv_path);
}

fn measure_proc_throughput_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<(f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>)> = (0..num_samples).map(|_| {
        measure_single_run_proc_mixed_intensity(
            base_workers, probe_intensity, cpu_iterations, io_iterations,
            duration_secs, io_perc, buffer_kb, io_buffer_kb, sleep_us, warmup_secs,
        )
    }).collect();
    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();

    // Pick per-worker data from the sample closest to median total throughput
    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].3.clone();

    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list), per_worker)
}

fn measure_single_run_proc_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let total_workers = base_workers + 1;
    let shm_name = format!("/saturator_{}_{}", std::process::id(), total_workers);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, total_workers);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();

    // Spawn base workers at intensity=1.0
    for i in 0..base_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg("1.0")
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .spawn()
            .expect("Failed to spawn base worker process");
        children.push(child);
    }

    // Spawn 1 probe worker at probe_intensity
    let child = Command::new(&exe)
        .arg("__worker")
        .arg(&shm_name)
        .arg(base_workers.to_string())
        .arg(cpu_iterations.to_string())
        .arg(io_iterations.to_string())
        .arg(io_perc.to_string())
        .arg(buffer_kb.to_string())
        .arg(io_buffer_kb.to_string())
        .arg(probe_intensity.to_string())
        .arg(sleep_us.to_string())
        .arg(total_workers.to_string())
        .spawn()
        .expect("Failed to spawn probe worker process");
    children.push(child);

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < total_workers as u64 {
        std::thread::sleep(Duration::from_millis(10));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    // Reset per-worker counters after warmup
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);

    for mut child in children {
        let _ = child.wait();
    }

    let cpu = region.cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;
    let io = region.io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    // Read per-worker counters
    let mut per_worker = Vec::with_capacity(total_workers);
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        per_worker.push((wc, wi, ws));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, total_workers);

    (cpu, io, metrics, per_worker)
}

fn run_saturator_split(
    thread_id: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    buffer_size: usize,
    io_buffer_size: usize,
    cpu_ops: Arc<AtomicU64>,
    io_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    intensity: f64,
    sleep_us: u64,
    pt_cpu: Arc<AtomicU64>,
    pt_io: Arc<AtomicU64>,
    pt_sleep: Arc<AtomicU64>,
) {
    use saturator::{cpu_work_with_buffer, io_work_with_id};

    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_buf = vec![0u8; io_buffer_size];

    let mut rng_state = thread_id as u64 + 12345;

    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;
    let mut local_sleep_ops = 0u64;

    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                local_sleep_ops += 1;
                if local_sleep_ops >= 100 {
                    pt_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
                    local_sleep_ops = 0;
                }
                continue;
            }
        }

        if rand_f64 < io_ratio {
            io_work_with_id(thread_id, io_iterations, &io_buf);
            local_io_ops += io_iterations as u64;
        } else {
            cpu_work_with_buffer(&cpu_buffer, cpu_iterations);
            local_cpu_ops += cpu_iterations as u64;
        }

        // Batch updates
        if (local_cpu_ops + local_io_ops) >= 100 {
            cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            pt_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
            pt_io.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    // Flush remaining
    if local_cpu_ops > 0 {
        cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
        pt_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
        pt_io.fetch_add(local_io_ops, Ordering::Relaxed);
    }
    if local_sleep_ops > 0 {
        pt_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
    }
}

// === Process-based slack experiment ===

fn measure_single_run_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let total_workers = baseline_workers + extra_workers;
    let shm_name = format!("/saturator_{}_{}", std::process::id(), total_workers);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, total_workers);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();

    for i in 0..baseline_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(baseline_io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .spawn()
            .expect("Failed to spawn baseline worker");
        children.push(child);
    }

    for i in 0..extra_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg((baseline_workers + i).to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(extra_io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .spawn()
            .expect("Failed to spawn extra worker");
        children.push(child);
    }

    while region.ready_count.load(Ordering::Relaxed) < total_workers as u64 {
        std::thread::sleep(Duration::from_millis(10));
    }

    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);
    for mut child in children {
        let _ = child.wait();
    }

    let mut baseline_cpu = 0.0_f64;
    let mut baseline_io = 0.0_f64;
    let mut extra_cpu = 0.0_f64;
    let mut extra_io = 0.0_f64;
    let mut per_worker = Vec::with_capacity(total_workers);

    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        per_worker.push((wc, wi, ws));
        if i < baseline_workers {
            baseline_cpu += wc;
            baseline_io += wi;
        } else {
            extra_cpu += wc;
            extra_io += wi;
        }
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, total_workers);
    (baseline_cpu, baseline_io, extra_cpu, extra_io, metrics, per_worker)
}

fn measure_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
) -> (f64, f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<_> = (0..num_samples).map(|_| {
        measure_single_run_proc_slack(
            baseline_workers, extra_workers, baseline_io_perc, extra_io_perc,
            cpu_iterations, io_iterations, duration_secs, buffer_kb, io_buffer_kb,
            intensity, sleep_us, warmup_secs,
        )
    }).collect();

    let b_cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let b_io_vals: Vec<f64>  = samples.iter().map(|s| s.1).collect();
    let e_cpu_vals: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let e_io_vals: Vec<f64>  = samples.iter().map(|s| s.3).collect();
    let metrics_list: Vec<_> = samples.iter().map(|s| s.4.clone()).collect();

    let median_baseline = median(&b_cpu_vals) + median(&b_io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            ((a.0 + a.1) - median_baseline).abs()
                .partial_cmp(&((b.0 + b.1) - median_baseline).abs()).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].5.clone();

    (
        median(&b_cpu_vals), median(&b_io_vals),
        median(&e_cpu_vals), median(&e_io_vals),
        stddev(&b_cpu_vals), stddev(&b_io_vals),
        proc_metrics::median_metrics(&metrics_list),
        per_worker,
    )
}

fn run_slack_proc_experiment(
    baseline_label: &str,
    baseline_io_perc: f64,
    track_io: bool,
    baseline_workers: usize,
    extra_io_perc: f64,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    println!("=== FINDING PROC SLACK ({} BASELINE) ===", baseline_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --max-workers={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb, params.max_workers,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 { print!(", --intensity={:.2}", params.intensity); }
    println!();
    println!("Baseline: {} {}-only processes", baseline_workers, baseline_label);
    println!("Adding processes at: {:.0}% I/O\n", extra_io_perc * 100.0);

    let sleep_us = calibration.cpu_us as u64;

    let (b_cpu, b_io, _, _, b0_cpu_std, b0_io_std, b0_metrics, b0_per_worker) = measure_proc_slack(
        baseline_workers, 0, baseline_io_perc, 0.0,
        calibration.cpu_iterations, calibration.io_iterations,
        params.duration_secs, params.buffer_kb, params.io_buffer_kb,
        params.samples, params.intensity, sleep_us, params.warmup_secs,
    );
    let baseline_throughput = if track_io { b_io } else { b_cpu };
    println!("Baseline: {:.0} {} ops/sec (cpu: {:.0}, io: {:.0})\n",
             baseline_throughput, baseline_label, b_cpu, b_io);

    let csv_base = format!("proc_slack_{}{}proc_adding_{}pct_io",
                           baseline_workers,
                           baseline_label.to_lowercase().replace("/", ""),
                           (extra_io_perc * 100.0) as u32);
    let run_dir = format!("{}_{}", csv_base, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("{} proc slack", baseline_label), params, &calibration, &[
        ("baseline_workers", format!("{}", baseline_workers)),
        ("baseline_io_perc", format!("{}", baseline_io_perc)),
        ("extra_io_perc", format!("{}", extra_io_perc)),
    ]);

    let filename = format!("{}/{}.csv", run_dir, csv_base);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_workers,total_workers,baseline_workers,extra_io_pct,baseline_cpu_ops,baseline_io_ops,extra_cpu_ops,extra_io_ops,total_ops,baseline_change_pct,baseline_cpu_stddev,baseline_io_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    // Write extra=0 baseline row
    writeln!(file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, baseline_workers, baseline_workers,
             (extra_io_perc * 100.0) as u32,
             b_cpu, b_io, 0.0_f64, 0.0_f64, b_cpu + b_io, 0.0,
             b0_cpu_std, b0_io_std,
             b0_metrics.to_csv_row()).unwrap();

    println!("  {:>5} {:>7} | {:>12} {:>12} | {:>12} {:>12} | {:>8} | {:>6} {:>6}",
             "extra", "total", "base_cpu/s", "base_io/s", "extra_cpu/s", "extra_io/s", "vs base", "cpu%", "io%");
    println!("  {}", "-".repeat(105));

    let mut per_worker_rows: Vec<(usize, Vec<(f64, f64, f64)>)> = vec![(baseline_workers, b0_per_worker)];

    for extra in (1..=params.max_workers).step_by(params.step) {
        let (b_cpu, b_io, e_cpu, e_io, b_cpu_std, b_io_std, metrics, per_worker) = measure_proc_slack(
            baseline_workers, extra, baseline_io_perc, extra_io_perc,
            calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, params.buffer_kb, params.io_buffer_kb,
            params.samples, params.intensity, sleep_us, params.warmup_secs,
        );

        let total_ops = b_cpu + b_io + e_cpu + e_io;
        let tracked = if track_io { b_io } else { b_cpu };
        let baseline_change = (tracked - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>5} {:>7} | {:>12.0} {:>12.0} | {:>12.0} {:>12.0} | {:>+7.1}% | {:>5.1}% {:>5.1}%",
                 extra, baseline_workers + extra, b_cpu, b_io, e_cpu, e_io, baseline_change,
                 metrics.cpu_pct, metrics.io_util_pct);

        writeln!(file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 extra, baseline_workers + extra, baseline_workers,
                 (extra_io_perc * 100.0) as u32,
                 b_cpu, b_io, e_cpu, e_io, total_ops, baseline_change,
                 b_cpu_std, b_io_std,
                 metrics.to_csv_row()).unwrap();

        per_worker_rows.push((baseline_workers + extra, per_worker));
    }

    {
        use std::io::Write as _;
        let pw_path = format!("{}/per_worker_{}.csv", run_dir, csv_base);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "extra_workers,total_workers,baseline_workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (total_workers, per_worker) in &per_worker_rows {
            let extra = total_workers - baseline_workers;
            for (wid, (wc, wi, ws)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2}",
                         extra, total_workers, baseline_workers, wid, wc, wi, ws, wc + wi).unwrap();
            }
        }
    }

    println!("\nResults written to: {}", filename);
}
