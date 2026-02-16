mod saturator;
mod visualize;
mod proc_metrics;

use saturator::{
    calibrate_operations_full, CalibrationResult, TuningParams,
    run_saturator, run_worker_process,
    create_shared_region, destroy_shared_region,
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
        println!("");
        println!("Options (for -proc variants):");
        println!("  --target-us <N>      Calibration target per-op in μs (default: 50)");
        println!("  --buffer-kb <N>      CPU work buffer size in KB (default: 100)");
        println!("  --max-workers <N>    Max worker count (default: parallelism * 16)");
        println!("  --duration <N>       Measurement duration in seconds (default: 5)");
        println!("  --samples <N>        Samples per data point, median taken (default: 3)");
        println!("  --step <N>           Worker count increment per data point (default: 1)");
        println!("");
        println!("Examples:");
        println!("  find-slack 4 100     - 4 CPU baseline, add 100% I/O threads");
        println!("  find-saturation-proc --buffer-kb 1024 --max-workers 100 --samples 7");
        return;
    }

    let experiment = &args[1];

    // Hidden worker subcommand — child process entry point
    if experiment == "__worker" {
        // Args: __worker <shm_name> <worker_id> <cpu_iters> <io_iters> <io_perc> <buffer_kb>
        let shm_name = &args[2];
        let worker_id: usize = args[3].parse().unwrap();
        let cpu_iters: usize = args[4].parse().unwrap();
        let io_iters: usize = args[5].parse().unwrap();
        let io_perc: f64 = args[6].parse().unwrap();
        let buffer_kb: usize = args[7].parse().unwrap();

        run_worker_process(shm_name, worker_id, cpu_iters, io_iters, io_perc, buffer_kb * 1024);
        return;
    }

    // Clean up any leftover files from previous runs
    cleanup_scratch_files();

    // Parse tuning parameters from optional flags
    let params = parse_tuning_params(&args);

    println!("Calibrating operations...");
    let calibration = calibrate_operations_full(&params);
    println!("Calibration: {} CPU iterations, {} I/O iterations",
             calibration.cpu_iterations, calibration.io_iterations);
    println!("Theoretical max: {:.0} CPU ops/s, {:.0} I/O ops/s per thread\n",
             calibration.cpu_ops_per_sec(), calibration.io_ops_per_sec());

    match experiment.as_str() {
        "find-saturation" => run_saturation_experiment(SaturationExperiment {
            label: "CPU",
            mode: Mode::Threads,
            io_perc: 0.0,
            csv_filename: "cpu_throughput_vs_threads.csv",
            recommendation: Some("find-io-slack"),
        }, calibration, &params),
        "find-io-saturation" => run_saturation_experiment(SaturationExperiment {
            label: "I/O",
            mode: Mode::Threads,
            io_perc: 1.0,
            csv_filename: "io_throughput_vs_threads.csv",
            recommendation: Some("find-cpu-slack"),
        }, calibration, &params),
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
        "find-saturation-proc" => run_saturation_experiment(SaturationExperiment {
            label: "CPU",
            mode: Mode::Procs,
            io_perc: 0.0,
            csv_filename: "proc_cpu_throughput_vs_workers.csv",
            recommendation: None,
        }, calibration, &params),
        "find-io-saturation-proc" => run_saturation_experiment(SaturationExperiment {
            label: "I/O",
            mode: Mode::Procs,
            io_perc: 1.0,
            csv_filename: "proc_io_throughput_vs_workers.csv",
            recommendation: None,
        }, calibration, &params),
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
            "--target-us" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.target_us = v;
                }
            }
            "--buffer-kb" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.buffer_kb = v;
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
    csv_filename: &'static str,
    recommendation: Option<&'static str>,
}

fn run_saturation_experiment(
    exp: SaturationExperiment,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    let (mode_label, worker_label) = match exp.mode {
        Mode::Threads => ("", "threads"),
        Mode::Procs => (" (PROCESS MODE)", "procs"),
    };

    println!("=== FINDING {} SATURATION POINT{} ===", exp.label, mode_label);
    if matches!(exp.mode, Mode::Procs) {
        println!("target_us={}, buffer={}KB, max_workers={}, duration={}s, step={}",
                 params.target_us, params.buffer_kb, params.max_workers, params.duration_secs, params.step);
    }
    println!("Adding {} {} until throughput {}\n",
             exp.label, worker_label,
             if matches!(exp.mode, Mode::Threads) { "plateaus" } else { "degrades" });

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = params.step;
    let iters = if exp.io_perc < 0.5 {
        calibration.cpu_iterations as f64
    } else {
        calibration.io_iterations as f64
    };

    let use_step = matches!(exp.mode, Mode::Procs);
    let mut worker_count = if use_step { params.step } else { 1 };
    let end = params.max_workers;

    while worker_count <= end {
        let (blocks_per_sec, throughput_stddev, metrics) = match exp.mode {
            Mode::Threads => measure_thread_throughput(
                worker_count, calibration.cpu_iterations, calibration.io_iterations,
                params.duration_secs, exp.io_perc, params.buffer_kb * 1024, params.samples,
            ),
            Mode::Procs => measure_proc_throughput(
                worker_count, calibration.cpu_iterations, calibration.io_iterations,
                params.duration_secs, exp.io_perc, params.buffer_kb, params.samples,
            ),
        };
        let throughput = blocks_per_sec * iters;
        let throughput_stddev_ops = throughput_stddev * iters;
        let throughput_per_worker = throughput / worker_count as f64;

        let width = if use_step { 3 } else { 2 };
        println!("  {:>width$} {}: {:12.0} ops/sec ({:10.0} per {}) [cpu:{:.1}% iow:{:.1}% ctx:{:.0}/s]",
                 worker_count, worker_label, throughput, throughput_per_worker,
                 if use_step { "proc" } else { "thread" },
                 metrics.cpu_pct, metrics.iowait_pct, metrics.ctx_switches_per_sec,
                 width = width);

        writer.add_saturation_point(worker_count, throughput, throughput_stddev_ops, metrics);

        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = worker_count;
        }

        if use_step {
            worker_count += params.step;
        } else {
            worker_count += 1;
        }
    }

    writer.write_saturation_csv(exp.csv_filename).unwrap();

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} {}", saturation_point, worker_label);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    if let Some(rec) = exp.recommendation {
        println!("\nRecommendation: Run `saturator {} {}` to find {} slack",
                 rec, saturation_point,
                 if exp.io_perc < 0.5 { "I/O" } else { "CPU" });
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
    println!("Baseline: {} {}-only threads", baseline_threads, exp.baseline_label);
    println!("Adding threads at: {:.0}% I/O\n", extra_io_ratio * 100.0);

    let cpu_iters = calibration.cpu_iterations as f64;
    let io_iters = calibration.io_iterations as f64;
    let buffer_size = params.buffer_kb * 1024;

    println!("Measuring baseline {} throughput...", exp.baseline_label);
    let (baseline_blocks, _baseline_stddev, _baseline_metrics) = measure_baseline(
        baseline_threads, exp.baseline_io_ratio,
        calibration.cpu_iterations, calibration.io_iterations,
        params.duration_secs, buffer_size, params.samples,
    );
    let baseline_iters = if exp.track_io { io_iters } else { cpu_iters };
    let baseline_throughput = baseline_blocks * baseline_iters;
    println!("Baseline: {:.0} {} ops/sec\n", baseline_throughput, exp.baseline_label);

    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_extra = logical_cores.max(baseline_threads * 2);
    let mut best_tracked_ops = baseline_throughput;
    let mut best_extra_count = 0;

    let filename = format!("slack_{}{}_adding_{}pct_io.csv",
                           baseline_threads,
                           exp.baseline_label.to_lowercase(),
                           (extra_io_ratio * 100.0) as u32);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,baseline_change_pct,cpu_ops_stddev,io_ops_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    let vs_label = if exp.track_io { "i/o vs base" } else { "cpu vs base" };
    println!("  {:>6} {:>8} | {:>14} {:>14} | {:>14} {:>12}",
             "extra", "threads", "cpu_ops/s", "io_ops/s", "total_ops/s", vs_label);
    println!("  {}", "-".repeat(90));

    for extra in 1..=max_extra {
        let (cpu_blocks, io_blocks, cpu_stddev, io_stddev, metrics) = measure_total_throughput(
            baseline_threads, extra, exp.baseline_io_ratio, extra_io_ratio,
            calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples
        );

        let cpu_ops = cpu_blocks * cpu_iters;
        let io_ops = io_blocks * io_iters;
        let total_ops = cpu_ops + io_ops;
        let cpu_ops_stddev = cpu_stddev * cpu_iters;
        let io_ops_stddev = io_stddev * io_iters;
        let tracked_ops = if exp.track_io { io_ops } else { cpu_ops };
        let baseline_change = (tracked_ops - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>6} {:>8} | {:>14.0} {:>14.0} | {:>14.0} {:>+11.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, baseline_change);

        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, baseline_change,
                 cpu_ops_stddev, io_ops_stddev,
                 metrics.to_csv_row()).unwrap();

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
    num_samples: usize,
) -> (f64, f64, proc_metrics::SystemMetrics) {
    let samples: Vec<(f64, proc_metrics::SystemMetrics)> = (0..num_samples).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_size)
    }).collect();
    let throughputs: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.1).collect();
    (median(&throughputs), stddev(&throughputs), proc_metrics::median_metrics(&metrics_list))
}

fn measure_proc_throughput(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    num_samples: usize,
) -> (f64, f64, proc_metrics::SystemMetrics) {
    let samples: Vec<(f64, proc_metrics::SystemMetrics)> = (0..num_samples).map(|_| {
        measure_single_run_proc(worker_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_kb)
    }).collect();
    let throughputs: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.1).collect();
    (median(&throughputs), stddev(&throughputs), proc_metrics::median_metrics(&metrics_list))
}

fn measure_single_run_proc(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
) -> (f64, proc_metrics::SystemMetrics) {
    use std::process::Command;

    let shm_name = format!("/saturator_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name);
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
            .spawn()
            .expect("Failed to spawn worker process");
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(10));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(1));
    region.total_ops.store(0, Ordering::Relaxed);

    let snap_before = proc_metrics::take_snapshot();
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);
    let ops = region.total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    // Reap children
    for mut child in children {
        let _ = child.wait();
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd);

    (ops, metrics)
}

fn measure_baseline(
    threads: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    num_samples: usize,
) -> (f64, f64, proc_metrics::SystemMetrics) {
    let samples: Vec<(f64, proc_metrics::SystemMetrics)> = (0..num_samples).map(|_| {
        let total_ops = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let mut handles = vec![];

        for i in 0..threads {
            let running = Arc::clone(&running);
            let ops = Arc::clone(&total_ops);

            let handle = std::thread::spawn(move || {
                run_saturator(i, io_ratio, cpu_iterations, io_iterations, buffer_size, ops, running);
            });
            handles.push(handle);
        }

        std::thread::sleep(Duration::from_secs(1));
        total_ops.store(0, Ordering::Relaxed);

        let snap_before = proc_metrics::take_snapshot();
        std::thread::sleep(Duration::from_secs(duration_secs));
        let snap_after = proc_metrics::take_snapshot();
        let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

        let result = total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

        running.store(false, Ordering::Relaxed);
        for handle in handles {
            handle.join().unwrap();
        }

        (result, metrics)
    }).collect();

    let throughputs: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let metrics_list: Vec<proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.1).collect();
    (median(&throughputs), stddev(&throughputs), proc_metrics::median_metrics(&metrics_list))
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn stddev(samples: &[f64]) -> f64 {
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt()
}

fn measure_single_run(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_size: usize,
) -> (f64, proc_metrics::SystemMetrics) {
    let total_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));
    let mut handles = vec![];

    for i in 0..thread_count {
        let running = Arc::clone(&running);
        let total_ops = Arc::clone(&total_ops);

        let handle = std::thread::spawn(move || {
            run_saturator(i, io_perc, cpu_iterations, io_iterations, buffer_size, total_ops, running);
        });
        handles.push(handle);
    }

    // Warmup: let threads run for 1 second, then reset counter
    std::thread::sleep(Duration::from_secs(1));
    total_ops.store(0, Ordering::Relaxed);

    let snap_before = proc_metrics::take_snapshot();
    // Now measure for the actual duration
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    let ops = total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    (ops, metrics)
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
    num_samples: usize,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics) {
    let results: Vec<(f64, f64, proc_metrics::SystemMetrics)> = (0..num_samples).map(|_| {
        measure_total_throughput_single(
            baseline_threads, extra_threads, baseline_io_ratio, extra_io_ratio,
            cpu_iterations, io_iterations, duration_secs, buffer_size,
        )
    }).collect();

    let cpu_vals: Vec<f64> = results.iter().map(|r| r.0).collect();
    let io_vals: Vec<f64> = results.iter().map(|r| r.1).collect();
    let metrics_list: Vec<proc_metrics::SystemMetrics> = results.into_iter().map(|r| r.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), proc_metrics::median_metrics(&metrics_list))
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
) -> (f64, f64, proc_metrics::SystemMetrics) {
    let cpu_ops = Arc::new(AtomicU64::new(0));
    let io_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));
    let mut handles = vec![];

    // Spawn baseline threads
    for i in 0..baseline_threads {
        let running = Arc::clone(&running);
        let cpu_ops = Arc::clone(&cpu_ops);
        let io_ops = Arc::clone(&io_ops);
        let io_ratio = baseline_io_ratio;

        let handle = std::thread::spawn(move || {
            run_saturator_split(i, io_ratio, cpu_iterations, io_iterations, buffer_size, cpu_ops, io_ops, running);
        });
        handles.push(handle);
    }

    // Spawn extra threads
    for i in 0..extra_threads {
        let running = Arc::clone(&running);
        let cpu_ops = Arc::clone(&cpu_ops);
        let io_ops = Arc::clone(&io_ops);
        let io_ratio = extra_io_ratio;

        let handle = std::thread::spawn(move || {
            run_saturator_split(baseline_threads + i, io_ratio, cpu_iterations, io_iterations, buffer_size, cpu_ops, io_ops, running);
        });
        handles.push(handle);
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(1));
    cpu_ops.store(0, Ordering::Relaxed);
    io_ops.store(0, Ordering::Relaxed);

    let snap_before = proc_metrics::take_snapshot();
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    let result = (
        cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
        io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
    );

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    (result.0, result.1, metrics)
}

fn run_saturator_split(
    thread_id: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    buffer_size: usize,
    cpu_ops: Arc<AtomicU64>,
    io_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
) {
    use saturator::{cpu_work_with_buffer, io_work_with_id};

    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();

    let mut rng_state = thread_id as u64 + 12345;

    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;

    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        if rand_f64 < io_ratio {
            io_work_with_id(thread_id, io_iterations);
            local_io_ops += 1;
        } else {
            cpu_work_with_buffer(&cpu_buffer, cpu_iterations);
            local_cpu_ops += 1;
        }

        // Batch updates
        if (local_cpu_ops + local_io_ops) % 100 == 0 {
            cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    // Flush remaining
    if local_cpu_ops > 0 {
        cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
    }
}
