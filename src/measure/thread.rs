use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::constants::*;
use crate::saturator::{run_saturator, cpu_work_with_buffer, io_work_with_id};
use crate::proc_metrics;
use super::aggregate_samples;

/// Collect multiple samples of thread-based throughput and return aggregated results.
pub fn measure_thread_throughput(
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
    let samples: Vec<_> = (0..num_samples).map(|_| {
        measure_single_run(thread_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_size, io_buffer_size, intensity, sleep_us, warmup_secs)
    }).collect();
    aggregate_samples(samples)
}

/// Run a single measurement of `thread_count` threads for `duration_secs`.
pub fn measure_single_run(
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

/// Measure baseline throughput with N threads at a given I/O ratio.
pub fn measure_baseline(
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

    aggregate_samples(samples)
}

/// Collect multiple samples of mixed baseline+extra thread throughput and return aggregated results.
pub fn measure_total_throughput(
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
    let results: Vec<_> = (0..num_samples).map(|_| {
        measure_total_throughput_single(
            baseline_threads, extra_threads, baseline_io_ratio, extra_io_ratio,
            cpu_iterations, io_iterations, duration_secs, buffer_size, io_buffer_size,
            intensity, sleep_us, warmup_secs,
        )
    }).collect();

    aggregate_samples(results)
}

/// Run a single measurement of baseline+extra threads with different I/O ratios.
pub fn measure_total_throughput_single(
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

/// Thread work loop that uses separate CPU/IO work functions (used for slack experiments).
pub fn run_saturator_split(
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
    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_buf = vec![0u8; io_buffer_size];

    let mut rng_state = thread_id as u64 + RNG_SEED_OFFSET;

    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;
    let mut local_sleep_ops = 0u64;

    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                local_sleep_ops += 1;
                if local_sleep_ops >= BATCH_FLUSH_THRESHOLD {
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
        if (local_cpu_ops + local_io_ops) >= BATCH_FLUSH_THRESHOLD {
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
