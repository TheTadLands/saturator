mod saturator;
mod metrics;

use saturator::{calibrate_operations, WorkloadConfig, run_saturator};
use metrics::monitor_throughput;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        println!("Usage: saturator <experiment>");
        println!("Experiments:");
        println!("  baseline          - Saturate CPU fully (baseline throughput)");
        println!("  add-cpu <N>       - Baseline + N additional CPU threads");
        println!("  add-io <N>        - Baseline + N I/O threads (find slack)");
        return;
    }
    
    let experiment = &args[1];
    
    println!("Calibrating operations...");
    let (cpu_iterations, io_size) = calibrate_operations();
    println!("Calibration: {} CPU iterations ≈ {} byte I/O op (~50μs each)\n", 
             cpu_iterations, io_size);
    
    match experiment.as_str() {
        "baseline" => run_baseline(cpu_iterations, io_size),
        "add-cpu" => {
            let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            run_add_cpu(cpu_iterations, io_size, n);
        },
        "add-io" => {
            let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            run_add_io(cpu_iterations, io_size, n);
        },
        _ => println!("Unknown experiment: {}", experiment),
    }
}

fn run_baseline(cpu_iterations: usize, io_size: usize) {
    println!("=== EXPERIMENT 1: CPU BASELINE ===");
    println!("Saturating all CPU cores with CPU-only work\n");
    
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    println!("Spawning {} CPU-only threads (one per core)", num_cpus);
    
    let config = WorkloadConfig {
        name: "CPU Baseline".to_string(),
        io_perc: 0.0,
    };
    
    run_experiment(config, num_cpus, cpu_iterations, io_size, 30);
}

fn run_add_cpu(cpu_iterations: usize, io_size: usize, additional: usize) {
    println!("=== EXPERIMENT 2: ADD CPU THREADS ===");
    println!("Testing if adding CPU work to saturated CPU decreases throughput\n");
    
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    println!("Spawning {} baseline CPU threads + {} additional CPU threads", 
             num_cpus, additional);
    println!("Expected: Throughput DECREASES due to contention\n");
    
    let config = WorkloadConfig {
        name: "CPU Saturated".to_string(),
        io_perc: 0.0,
    };
    
    run_experiment(config, num_cpus + additional, cpu_iterations, io_size, 30);
}

fn run_add_io(cpu_iterations: usize, io_size: usize, io_threads: usize) {
    println!("=== EXPERIMENT 3: ADD I/O THREADS ===");
    println!("Testing how much I/O work can run alongside CPU saturation\n");
    
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    
    println!("Spawning {} CPU threads + {} I/O threads", num_cpus, io_threads);
    println!("Expected: Throughput INCREASES if I/O doesn't bottleneck CPU\n");
    
    let total_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));
    let mut handles = vec![];
    
    for i in 0..num_cpus {
        let running = Arc::clone(&running);
        let total_ops = Arc::clone(&total_ops);
        let config = WorkloadConfig {
            name: "CPU".to_string(),
            io_perc: 0.0,
        };
        
        let handle = std::thread::spawn(move || {
            run_saturator(i, config, cpu_iterations, io_size, total_ops, running);
        });
        handles.push(handle);
    }
    
    for i in num_cpus..(num_cpus + io_threads) {
        let running = Arc::clone(&running);
        let total_ops = Arc::clone(&total_ops);
        let config = WorkloadConfig {
            name: "I/O".to_string(),
            io_perc: 1.0,
        };
        
        let handle = std::thread::spawn(move || {
            run_saturator(i, config, cpu_iterations, io_size, total_ops, running);
        });
        handles.push(handle);
    }
    
    let monitor_running = Arc::clone(&running);
    let monitor_ops = Arc::clone(&total_ops);
    let monitor = std::thread::spawn(move || {
        monitor_throughput(monitor_ops, monitor_running);
    });
    
    std::thread::sleep(Duration::from_secs(30));
    running.store(false, Ordering::Relaxed);
    
    for handle in handles {
        handle.join().unwrap();
    }
    monitor.join().unwrap();
    
    let final_ops = total_ops.load(Ordering::Relaxed);
    println!("\n=== RESULTS ===");
    println!("Total operations: {}", final_ops);
    println!("Average throughput: {:.0} ops/sec", final_ops as f64 / 30.0);
}

fn run_experiment(
    config: WorkloadConfig,
    thread_count: usize,
    cpu_iterations: usize,
    io_size: usize,
    duration_secs: u64,
) {
    let total_ops = Arc::new(AtomicU64::new(0));
    let running = Arc::new(AtomicBool::new(true));
    let mut handles = vec![];
    
    for i in 0..thread_count {
        let running = Arc::clone(&running);
        let total_ops = Arc::clone(&total_ops);
        let cfg = config.clone();
        
        let handle = std::thread::spawn(move || {
            run_saturator(i, cfg, cpu_iterations, io_size, total_ops, running);
        });
        handles.push(handle);
    }
    
    let monitor_running = Arc::clone(&running);
    let monitor_ops = Arc::clone(&total_ops);
    let monitor = std::thread::spawn(move || {
        monitor_throughput(monitor_ops, monitor_running);
    });
    
    std::thread::sleep(Duration::from_secs(duration_secs));
    running.store(false, Ordering::Relaxed);
    
    for handle in handles {
        handle.join().unwrap();
    }
    monitor.join().unwrap();
    
    let final_ops = total_ops.load(Ordering::Relaxed);
    println!("\n=== RESULTS ===");
    println!("Total operations: {}", final_ops);
    println!("Average throughput: {:.0} ops/sec", final_ops as f64 / duration_secs as f64);
}