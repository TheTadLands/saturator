mod saturator;
mod metrics;
mod visualize;

use saturator::{
    calibrate_operations_full, CalibrationResult, TuningParams, WorkloadConfig,
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
        "find-saturation" => find_cpu_saturation(calibration, &params),
        "find-io-saturation" => find_io_saturation(calibration, &params),
        "find-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(100.0);
            find_cpu_slack(calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-io-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            find_io_slack(calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-saturation-proc" => find_cpu_saturation_proc(calibration, &params),
        "find-io-saturation-proc" => find_io_saturation_proc(calibration, &params),
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

// === Thread-based experiments (existing) ===

fn find_cpu_saturation(calibration: CalibrationResult, params: &TuningParams) {
    println!("=== FINDING CPU SATURATION POINT ===");
    println!("Adding CPU threads until throughput plateaus\n");

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    let iters = calibration.cpu_iterations as f64;
    let buffer_size = params.buffer_kb * 1024;

    for thread_count in 1..=params.max_workers {
        let blocks_per_sec = measure_cpu_throughput(thread_count, calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples);
        let throughput = blocks_per_sec * iters;
        let throughput_per_thread = throughput / thread_count as f64;

        println!("  {:2} threads: {:12.0} ops/sec ({:10.0} per thread)",
                 thread_count, throughput, throughput_per_thread);

        writer.add_saturation_point(thread_count, throughput);

        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = thread_count;
        }
    }

    writer.write_saturation_csv("cpu_throughput_vs_threads.csv").unwrap();

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} threads", saturation_point);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    println!("\nRecommendation: Run `saturator find-io-slack {}` to find I/O slack", saturation_point);
}

fn find_io_saturation(calibration: CalibrationResult, params: &TuningParams) {
    println!("=== FINDING I/O SATURATION POINT ===");
    println!("Adding I/O threads until throughput plateaus\n");

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    let iters = calibration.io_iterations as f64;
    let buffer_size = params.buffer_kb * 1024;

    for thread_count in 1..=params.max_workers {
        let blocks_per_sec = measure_io_throughput(thread_count, calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples);
        let throughput = blocks_per_sec * iters;
        let throughput_per_thread = throughput / thread_count as f64;

        println!("  {:2} threads: {:12.0} ops/sec ({:10.0} per thread)",
                 thread_count, throughput, throughput_per_thread);

        writer.add_saturation_point(thread_count, throughput);

        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = thread_count;
        }
    }

    writer.write_saturation_csv("io_throughput_vs_threads.csv").unwrap();

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} threads", saturation_point);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    println!("\nRecommendation: Run `saturator find-cpu-slack {}` to find CPU slack", saturation_point);
}

fn find_cpu_slack(
    calibration: CalibrationResult,
    params: &TuningParams,
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK (CPU BASELINE) ===");
    println!("Baseline: {} CPU-only threads", baseline_threads);
    println!("Adding threads at: {:.0}% I/O\n", extra_io_ratio * 100.0);

    let cpu_iters = calibration.cpu_iterations as f64;
    let io_iters = calibration.io_iterations as f64;
    let buffer_size = params.buffer_kb * 1024;

    println!("Measuring baseline CPU throughput...");
    let baseline_blocks = measure_baseline(baseline_threads, 0.0, calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples);
    let baseline_throughput = baseline_blocks * cpu_iters;
    println!("Baseline: {:.0} CPU ops/sec\n", baseline_throughput);

    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_extra = logical_cores.max(baseline_threads * 2);
    let mut best_cpu_ops = baseline_throughput;
    let mut best_extra_count = 0;

    let filename = format!("slack_{}cpu_adding_{}pct_io.csv",
                           baseline_threads,
                           (extra_io_ratio * 100.0) as u32);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,baseline_change_pct").unwrap();

    println!("  {:>6} {:>8} | {:>14} {:>14} | {:>14} {:>12}",
             "extra", "threads", "cpu_ops/s", "io_ops/s", "total_ops/s", "cpu vs base");
    println!("  {}", "-".repeat(90));

    for extra in 1..=max_extra {
        let (cpu_blocks, io_blocks) = measure_total_throughput(
            baseline_threads, extra, 0.0, extra_io_ratio,
            calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples
        );

        let cpu_ops = cpu_blocks * cpu_iters;
        let io_ops = io_blocks * io_iters;
        let total_ops = cpu_ops + io_ops;
        let baseline_change = (cpu_ops - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>6} {:>8} | {:>14.0} {:>14.0} | {:>14.0} {:>+11.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, baseline_change);

        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2}",
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, baseline_change).unwrap();

        if cpu_ops > best_cpu_ops {
            best_cpu_ops = cpu_ops;
            best_extra_count = extra;
        }
    }

    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} CPU ops/sec", baseline_threads, baseline_throughput);
    println!("Best CPU: {} threads = {:.0} CPU ops/sec ({:+.1}%)",
             baseline_threads + best_extra_count, best_cpu_ops,
             (best_cpu_ops - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}

fn find_io_slack(
    calibration: CalibrationResult,
    params: &TuningParams,
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK (I/O BASELINE) ===");
    println!("Baseline: {} I/O-only threads", baseline_threads);
    println!("Adding threads at: {:.0}% I/O\n", extra_io_ratio * 100.0);

    let cpu_iters = calibration.cpu_iterations as f64;
    let io_iters = calibration.io_iterations as f64;
    let buffer_size = params.buffer_kb * 1024;

    println!("Measuring baseline I/O throughput...");
    let baseline_blocks = measure_baseline(baseline_threads, 1.0, calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples);
    let baseline_throughput = baseline_blocks * io_iters;
    println!("Baseline: {:.0} I/O ops/sec\n", baseline_throughput);

    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_extra = logical_cores.max(baseline_threads * 2);
    let mut best_io_ops = baseline_throughput;
    let mut best_extra_count = 0;

    let filename = format!("slack_{}io_adding_{}pct_io.csv",
                           baseline_threads,
                           (extra_io_ratio * 100.0) as u32);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,baseline_change_pct").unwrap();

    println!("  {:>6} {:>8} | {:>14} {:>14} | {:>14} {:>12}",
             "extra", "threads", "cpu_ops/s", "io_ops/s", "total_ops/s", "i/o vs base");
    println!("  {}", "-".repeat(90));

    for extra in 1..=max_extra {
        let (cpu_blocks, io_blocks) = measure_total_throughput(
            baseline_threads, extra, 1.0, extra_io_ratio,
            calibration.cpu_iterations, calibration.io_iterations, params.duration_secs, buffer_size, params.samples
        );

        let cpu_ops = cpu_blocks * cpu_iters;
        let io_ops = io_blocks * io_iters;
        let total_ops = cpu_ops + io_ops;
        let baseline_change = (io_ops - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>6} {:>8} | {:>14.0} {:>14.0} | {:>14.0} {:>+11.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, baseline_change);

        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2}",
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, baseline_change).unwrap();

        if io_ops > best_io_ops {
            best_io_ops = io_ops;
            best_extra_count = extra;
        }
    }

    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} I/O ops/sec", baseline_threads, baseline_throughput);
    println!("Best I/O: {} threads = {:.0} I/O ops/sec ({:+.1}%)",
             baseline_threads + best_extra_count, best_io_ops,
             (best_io_ops - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}

// === Process-based experiments (new) ===

fn find_cpu_saturation_proc(calibration: CalibrationResult, params: &TuningParams) {
    println!("=== FINDING CPU SATURATION POINT (PROCESS MODE) ===");
    println!("target_us={}, buffer={}KB, max_workers={}, duration={}s",
             params.target_us, params.buffer_kb, params.max_workers, params.duration_secs);
    println!("Adding worker processes until throughput degrades\n");

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    let iters = calibration.cpu_iterations as f64;

    for worker_count in 1..=params.max_workers {
        let blocks_per_sec = measure_proc_throughput(
            worker_count, calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, 0.0, params.buffer_kb, params.samples,
        );
        let throughput = blocks_per_sec * iters;
        let throughput_per_worker = throughput / worker_count as f64;

        println!("  {:3} procs: {:12.0} ops/sec ({:10.0} per proc)",
                 worker_count, throughput, throughput_per_worker);

        writer.add_saturation_point(worker_count, throughput);

        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = worker_count;
        }
    }

    writer.write_saturation_csv("proc_cpu_throughput_vs_workers.csv").unwrap();

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} processes", saturation_point);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
}

fn find_io_saturation_proc(calibration: CalibrationResult, params: &TuningParams) {
    println!("=== FINDING I/O SATURATION POINT (PROCESS MODE) ===");
    println!("target_us={}, buffer={}KB, max_workers={}, duration={}s",
             params.target_us, params.buffer_kb, params.max_workers, params.duration_secs);
    println!("Adding worker processes until throughput degrades\n");

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    let iters = calibration.io_iterations as f64;

    for worker_count in 1..=params.max_workers {
        let blocks_per_sec = measure_proc_throughput(
            worker_count, calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, 1.0, params.buffer_kb, params.samples,
        );
        let throughput = blocks_per_sec * iters;
        let throughput_per_worker = throughput / worker_count as f64;

        println!("  {:3} procs: {:12.0} ops/sec ({:10.0} per proc)",
                 worker_count, throughput, throughput_per_worker);

        writer.add_saturation_point(worker_count, throughput);

        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = worker_count;
        }
    }

    writer.write_saturation_csv("proc_io_throughput_vs_workers.csv").unwrap();

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} processes", saturation_point);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
}

// === Measurement helpers ===

fn measure_proc_throughput(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    num_samples: usize,
) -> f64 {
    let samples: Vec<f64> = (0..num_samples).map(|_| {
        measure_single_run_proc(worker_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_kb)
    }).collect();
    median(&samples)
}

fn measure_single_run_proc(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
) -> f64 {
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

    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));

    region.running.store(false, Ordering::Relaxed);
    let ops = region.total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    // Reap children
    for mut child in children {
        let _ = child.wait();
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd);

    ops
}

fn measure_baseline(
    threads: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    num_samples: usize,
) -> f64 {
    let samples: Vec<f64> = (0..num_samples).map(|_| {
        let total_ops = Arc::new(AtomicU64::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let mut handles = vec![];

        for i in 0..threads {
            let running = Arc::clone(&running);
            let ops = Arc::clone(&total_ops);
            let config = WorkloadConfig {
                name: format!("baseline-{}", i),
                io_perc: io_ratio,
            };

            let handle = std::thread::spawn(move || {
                run_saturator(i, config, cpu_iterations, io_iterations, buffer_size, ops, running);
            });
            handles.push(handle);
        }

        std::thread::sleep(Duration::from_secs(1));
        total_ops.store(0, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(duration_secs));
        let result = total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

        running.store(false, Ordering::Relaxed);
        for handle in handles {
            handle.join().unwrap();
        }

        result
    }).collect();

    median(&samples)
}

fn measure_cpu_throughput(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    num_samples: usize,
) -> f64 {
    let samples: Vec<f64> = (0..num_samples).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, 0.0, buffer_size)
    }).collect();
    median(&samples)
}

fn measure_io_throughput(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_size: usize,
    num_samples: usize,
) -> f64 {
    let samples: Vec<f64> = (0..num_samples).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, 1.0, buffer_size)
    }).collect();
    median(&samples)
}

fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

fn measure_single_run(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_size: usize,
) -> f64 {
    let total_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));
    let mut handles = vec![];

    for i in 0..thread_count {
        let running = Arc::clone(&running);
        let total_ops = Arc::clone(&total_ops);
        let config = WorkloadConfig {
            name: if io_perc < 0.5 { "CPU".to_string() } else { "I/O".to_string() },
            io_perc,
        };

        let handle = std::thread::spawn(move || {
            run_saturator(i, config, cpu_iterations, io_iterations, buffer_size, total_ops, running);
        });
        handles.push(handle);
    }

    // Warmup: let threads run for 1 second, then reset counter
    std::thread::sleep(Duration::from_secs(1));
    total_ops.store(0, Ordering::Relaxed);

    // Now measure for the actual duration
    std::thread::sleep(Duration::from_secs(duration_secs));
    let ops = total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64;

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    ops
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
) -> (f64, f64) {
    let results: Vec<(f64, f64)> = (0..num_samples).map(|_| {
        measure_total_throughput_single(
            baseline_threads, extra_threads, baseline_io_ratio, extra_io_ratio,
            cpu_iterations, io_iterations, duration_secs, buffer_size,
        )
    }).collect();

    let mut cpu_vals: Vec<f64> = results.iter().map(|r| r.0).collect();
    let mut io_vals: Vec<f64> = results.iter().map(|r| r.1).collect();
    cpu_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    io_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (cpu_vals[cpu_vals.len() / 2], io_vals[io_vals.len() / 2])
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
) -> (f64, f64) {
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

    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let result = (
        cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
        io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
    );

    running.store(false, Ordering::Relaxed);
    for handle in handles {
        handle.join().unwrap();
    }

    result
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
