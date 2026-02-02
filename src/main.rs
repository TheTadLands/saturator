mod saturator;
mod metrics;
mod visualize;

use saturator::{calibrate_operations, WorkloadConfig, run_saturator};
use visualize::ResultsWriter;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        println!("Usage: saturator <experiment>");
        println!("Experiments:");
        println!("  find-saturation              - Find CPU saturation point");
        println!("  find-io-saturation           - Find I/O saturation point");
        println!("  find-slack <N> <extra_io%>   - N CPU baseline threads, add threads at extra_io%");
        println!("  find-io-slack <N> <extra_io%> - N I/O baseline threads, add threads at extra_io%");
        println!("");
        println!("Examples:");
        println!("  find-slack 4 100    - 4 CPU baseline, add 100% I/O threads");
        println!("  find-slack 4 50     - 4 CPU baseline, add 50% I/O threads");
        println!("  find-io-slack 4 0   - 4 I/O baseline, add CPU-only threads");
        println!("  find-io-slack 4 50  - 4 I/O baseline, add 50% I/O threads");
        return;
    }
    
    // Clean up any leftover files from previous runs
    cleanup_scratch_files();
    
    let experiment = &args[1];
    
    println!("Calibrating operations...");
    let (cpu_iterations, io_iterations) = calibrate_operations();
    println!("Calibration: {} CPU iterations, {} I/O iterations (~50μs each)\n", 
             cpu_iterations, io_iterations);
    
    match experiment.as_str() {
        "find-saturation" => find_cpu_saturation(cpu_iterations, io_iterations),
        "find-io-saturation" => find_io_saturation(cpu_iterations, io_iterations),
        "find-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(100.0);
            find_cpu_slack(cpu_iterations, io_iterations, baseline, extra_io_pct / 100.0);
        },
        "find-io-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            find_io_slack(cpu_iterations, io_iterations, baseline, extra_io_pct / 100.0);
        },
        _ => println!("Unknown experiment: {}", experiment),
    }
    
    // Clean up after run
    cleanup_scratch_files();
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

fn find_cpu_saturation(cpu_iterations: usize, io_iterations: usize) {
    println!("=== FINDING CPU SATURATION POINT ===");
    println!("Adding CPU threads until throughput plateaus");
    println!("(1 op = 1 work unit calibrated to ~50μs)\n");
    
    let max_threads = std::thread::available_parallelism()
        .map(|n| n.get() * 4)
        .unwrap_or(12);
    
    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    
    for thread_count in 1..=max_threads {
        let throughput = measure_cpu_throughput(thread_count, cpu_iterations, io_iterations, 5);
        let throughput_per_thread = throughput / thread_count as f64;
        
        // Calculate % of best throughput
        let pct_of_best = if best_throughput > 0.0 { 
            throughput / best_throughput * 100.0 
        } else { 
            100.0 
        };
        
        println!("  {:2} threads: {:8.0} ops/sec ({:6.0} per thread) [{:5.1}% of peak]", 
                 thread_count, throughput, throughput_per_thread, pct_of_best);
        
        writer.add_saturation_point(thread_count, throughput);
        
        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = thread_count;
        }
    }
    
    writer.write_saturation_csv("cpu_saturation.csv").unwrap();
    
    println!("\n=== RESULTS ===");
    println!("Saturation point: {} threads (peak throughput)", saturation_point);
    println!("Peak throughput: {:.0} ops/sec", best_throughput);
    println!("Plateau range: threads maintaining >90% of peak");
    println!("\nRecommendation: Run `saturator find-io-slack {}` to find I/O slack", saturation_point);
}

fn find_io_saturation(cpu_iterations: usize, io_iterations: usize) {
    println!("=== FINDING I/O SATURATION POINT ===");
    println!("Adding I/O threads until throughput plateaus");
    println!("(1 op = 1 work unit calibrated to ~50μs)\n");
    
    let max_threads = std::thread::available_parallelism()
        .map(|n| n.get() * 4)
        .unwrap_or(12);
    
    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = 1;
    
    for thread_count in 1..=max_threads {
        let throughput = measure_io_throughput(thread_count, cpu_iterations, io_iterations, 5);
        let throughput_per_thread = throughput / thread_count as f64;
        
        let pct_of_best = if best_throughput > 0.0 { 
            throughput / best_throughput * 100.0 
        } else { 
            100.0 
        };
        
        println!("  {:2} threads: {:8.0} ops/sec ({:6.0} per thread) [{:5.1}% of peak]", 
                 thread_count, throughput, throughput_per_thread, pct_of_best);
        
        writer.add_saturation_point(thread_count, throughput);
        
        if throughput > best_throughput {
            best_throughput = throughput;
            saturation_point = thread_count;
        }
    }
    
    writer.write_saturation_csv("io_saturation.csv").unwrap();
    
    println!("\n=== RESULTS ===");
    println!("Saturation point: {} threads (peak throughput)", saturation_point);
    println!("Peak throughput: {:.0} ops/sec", best_throughput);
    println!("Plateau range: threads maintaining >90% of peak");
    println!("\nRecommendation: Run `saturator find-cpu-slack {}` to find CPU slack", saturation_point);
}

fn find_cpu_slack(
    cpu_iterations: usize, 
    io_iterations: usize, 
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK (CPU BASELINE) ===");
    println!("Baseline: {} CPU-only threads", baseline_threads);
    println!("Adding threads at: {:.0}% I/O", extra_io_ratio * 100.0);
    println!("(1 op = 1 work unit calibrated to ~50μs)\n");
    
    println!("Measuring baseline CPU throughput...");
    let baseline_throughput = measure_baseline(baseline_threads, 0.0, cpu_iterations, io_iterations, 5);
    println!("Baseline: {:.0} ops/sec (all CPU)\n", baseline_throughput);
    
    let max_extra = baseline_threads * 4;
    let mut best_total = baseline_throughput;
    let mut best_extra_count = 0;
    
    let filename = format!("cpu_slack_{}t_extra{}io.csv", 
                           baseline_threads,
                           (extra_io_ratio * 100.0) as u32);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,throughput_change_pct").unwrap();
    
    println!("  {:>6} {:>8} | {:>12} {:>12} | {:>12} {:>10}",
             "extra", "threads", "cpu_ops/s", "io_ops/s", "total_ops/s", "vs baseline");
    println!("  {}", "-".repeat(75));
    
    for extra in 1..=max_extra {
        let (cpu_ops, io_ops) = measure_total_throughput(
            baseline_threads, extra, 0.0, extra_io_ratio,
            cpu_iterations, io_iterations, 5
        );
        
        let total_ops = cpu_ops + io_ops;
        let throughput_change = (total_ops - baseline_throughput) / baseline_throughput * 100.0;
        
        println!("  {:>6} {:>8} | {:>12.0} {:>12.0} | {:>12.0} {:>+9.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, throughput_change);
        
        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2}", 
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, throughput_change).unwrap();
        
        if total_ops > best_total {
            best_total = total_ops;
            best_extra_count = extra;
        }
        
        if total_ops < baseline_throughput * 0.8 {
            println!("\n  → Total throughput dropped >20% below baseline, stopping.");
            break;
        }
    }
    
    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} ops/sec", baseline_threads, baseline_throughput);
    println!("Best: {} threads = {:.0} ops/sec ({:+.1}%)", 
             baseline_threads + best_extra_count, best_total,
             (best_total - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}

fn find_io_slack(
    cpu_iterations: usize, 
    io_iterations: usize, 
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK (I/O BASELINE) ===");
    println!("Baseline: {} I/O-only threads", baseline_threads);
    println!("Adding threads at: {:.0}% I/O", extra_io_ratio * 100.0);
    println!("(1 op = 1 work unit calibrated to ~50μs)\n");
    
    println!("Measuring baseline I/O throughput...");
    let baseline_throughput = measure_baseline(baseline_threads, 1.0, cpu_iterations, io_iterations, 5);
    println!("Baseline: {:.0} ops/sec (all I/O)\n", baseline_throughput);
    
    let max_extra = baseline_threads * 4;
    let mut best_total = baseline_throughput;
    let mut best_extra_count = 0;
    
    let filename = format!("io_slack_{}t_extra{}io.csv", 
                           baseline_threads,
                           (extra_io_ratio * 100.0) as u32);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,throughput_change_pct").unwrap();
    
    println!("  {:>6} {:>8} | {:>12} {:>12} | {:>12} {:>10}",
             "extra", "threads", "cpu_ops/s", "io_ops/s", "total_ops/s", "vs baseline");
    println!("  {}", "-".repeat(75));
    
    for extra in 1..=max_extra {
        let (cpu_ops, io_ops) = measure_total_throughput(
            baseline_threads, extra, 1.0, extra_io_ratio,
            cpu_iterations, io_iterations, 5
        );
        
        let total_ops = cpu_ops + io_ops;
        let throughput_change = (total_ops - baseline_throughput) / baseline_throughput * 100.0;
        
        println!("  {:>6} {:>8} | {:>12.0} {:>12.0} | {:>12.0} {:>+9.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, throughput_change);
        
        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2}", 
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, throughput_change).unwrap();
        
        if total_ops > best_total {
            best_total = total_ops;
            best_extra_count = extra;
        }
        
        if total_ops < baseline_throughput * 0.8 {
            println!("\n  → Total throughput dropped >20% below baseline, stopping.");
            break;
        }
    }
    
    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} ops/sec", baseline_threads, baseline_throughput);
    println!("Best: {} threads = {:.0} ops/sec ({:+.1}%)", 
             baseline_threads + best_extra_count, best_total,
             (best_total - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}

fn measure_baseline(
    threads: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
) -> f64 {
    let samples: Vec<f64> = (0..3).map(|_| {
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
                run_saturator(i, config, cpu_iterations, io_iterations, ops, running);
            });
            handles.push(handle);
        }
        
        std::thread::sleep(Duration::from_secs(1));
        total_ops.store(0, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(duration_secs));
        running.store(false, Ordering::Relaxed);
        
        for handle in handles {
            handle.join().unwrap();
        }
        
        total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64
    }).collect();
    
    median(&samples)
}

fn measure_cpu_throughput(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
) -> f64 {
    // Take multiple samples and return median for stability
    let samples: Vec<f64> = (0..3).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, 0.0)
    }).collect();
    
    median(&samples)
}

fn measure_io_throughput(
    thread_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
) -> f64 {
    // Take multiple samples and return median for stability
    let samples: Vec<f64> = (0..3).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, 1.0)
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
            run_saturator(i, config, cpu_iterations, io_iterations, total_ops, running);
        });
        handles.push(handle);
    }
    
    // Warmup: let threads run for 1 second, then reset counter
    std::thread::sleep(Duration::from_secs(1));
    total_ops.store(0, Ordering::Relaxed);
    
    // Now measure for the actual duration
    std::thread::sleep(Duration::from_secs(duration_secs));
    running.store(false, Ordering::Relaxed);
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    total_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64
}

fn measure_total_throughput(
    baseline_threads: usize,
    extra_threads: usize,
    baseline_io_ratio: f64,
    extra_io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
) -> (f64, f64) {  // Returns (total_cpu_ops, total_io_ops)
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
            run_saturator_split(i, io_ratio, cpu_iterations, io_iterations, cpu_ops, io_ops, running);
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
            run_saturator_split(baseline_threads + i, io_ratio, cpu_iterations, io_iterations, cpu_ops, io_ops, running);
        });
        handles.push(handle);
    }
    
    // Warmup
    std::thread::sleep(Duration::from_secs(1));
    cpu_ops.store(0, Ordering::Relaxed);
    io_ops.store(0, Ordering::Relaxed);
    
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    running.store(false, Ordering::Relaxed);
    
    for handle in handles {
        handle.join().unwrap();
    }
    
    (
        cpu_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
        io_ops.load(Ordering::Relaxed) as f64 / duration_secs as f64,
    )
}

fn run_saturator_split(
    thread_id: usize,
    io_ratio: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    cpu_ops: Arc<AtomicU64>,
    io_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
) {
    use saturator::{cpu_work, io_work_with_id};
    
    // Use same RNG approach as run_saturator for consistency
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
            cpu_work(cpu_iterations);
            local_cpu_ops += 1;
        }
        
        // Batch updates like run_saturator does
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