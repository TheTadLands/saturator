use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::saturator::{CalibrationResult, TuningParams, run_saturator, now_ns};
use crate::proc_metrics;
use super::aggregate_samples;

/// Collect multiple samples of thread-based throughput and return aggregated results.
pub fn measure_thread_throughput(
    thread_count: usize,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    measure_total_throughput(thread_count, 0, io_perc, 0.0, calibration, params)
}

/// Collect multiple samples of mixed baseline+extra thread throughput and return aggregated results.
pub fn measure_total_throughput(
    baseline_threads: usize,
    extra_threads: usize,
    baseline_io_ratio: f64,
    extra_io_ratio: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let results: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run(
            baseline_threads, extra_threads, baseline_io_ratio, extra_io_ratio,
            calibration, params,
        )
    }).collect();

    aggregate_samples(results)
}

/// Run a single measurement of baseline+extra threads with different I/O ratios.
/// When extra_threads == 0, this is a uniform measurement of baseline_threads at baseline_io_ratio.
fn measure_single_run(
    baseline_threads: usize,
    extra_threads: usize,
    baseline_io_ratio: f64,
    extra_io_ratio: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let total_threads = baseline_threads + extra_threads;
    let deadline_ns = Arc::new(AtomicU64::new(0));

    // Per-thread counters: [cpu, io, sleep, errors] per thread
    let per_thread: Vec<[Arc<AtomicU64>; 4]> = (0..total_threads)
        .map(|_| [Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0))])
        .collect();

    let mut handles = vec![];

    // Spawn baseline threads
    for (i, pt) in per_thread[..baseline_threads].iter().enumerate() {
        let deadline_ns = Arc::clone(&deadline_ns);
        let pt_cpu    = Arc::clone(&pt[0]);
        let pt_io     = Arc::clone(&pt[1]);
        let pt_sleep  = Arc::clone(&pt[2]);
        let pt_errors = Arc::clone(&pt[3]);
        let params = params.clone();
        let calibration = *calibration;

        let handle = std::thread::spawn(move || {
            run_saturator(i, baseline_io_ratio, deadline_ns, pt_cpu, pt_io, pt_sleep, pt_errors, params, calibration);
        });
        handles.push(handle);
    }

    // Spawn extra threads (if any)
    for (i, pt) in per_thread[baseline_threads..].iter().enumerate() {
        let deadline_ns = Arc::clone(&deadline_ns);
        let pt_cpu    = Arc::clone(&pt[0]);
        let pt_io     = Arc::clone(&pt[1]);
        let pt_sleep  = Arc::clone(&pt[2]);
        let pt_errors = Arc::clone(&pt[3]);
        let params = params.clone();
        let calibration = *calibration;

        let handle = std::thread::spawn(move || {
            run_saturator(baseline_threads + i, extra_io_ratio, deadline_ns, pt_cpu, pt_io, pt_sleep, pt_errors, params, calibration);
        });
        handles.push(handle);
    }

    // Warmup: let threads run, then reset counters
    std::thread::sleep(Duration::from_secs(params.warmup_secs));
    for pt in &per_thread {
        pt[0].store(0, Ordering::Relaxed);
        pt[1].store(0, Ordering::Relaxed);
        pt[2].store(0, Ordering::Relaxed);
        // Don't reset errors — they're cumulative diagnostics
    }

    let snap_before = proc_metrics::take_snapshot();
    let deadline = now_ns() + params.duration_secs * 1_000_000_000;
    deadline_ns.store(deadline, Ordering::Relaxed);
    // Now measure for the actual duration — workers self-terminate when clock passes deadline
    std::thread::sleep(Duration::from_secs(params.duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, params.duration_secs as f64);

    for handle in handles {
        handle.join().unwrap();
    }

    let per_thread_data: Vec<(f64, f64, f64, u64)> = per_thread.iter()
        .map(|pt| (
            pt[0].load(Ordering::Relaxed) as f64 / params.duration_secs as f64,
            pt[1].load(Ordering::Relaxed) as f64 / params.duration_secs as f64,
            pt[2].load(Ordering::Relaxed) as f64 / params.duration_secs as f64,
            pt[3].load(Ordering::Relaxed),
        ))
        .collect();

    // Aggregate from per-thread counters (same approach as process mode)
    let cpu: f64 = per_thread_data.iter().map(|(c, _, _, _)| c).sum();
    let io: f64 = per_thread_data.iter().map(|(_, i, _, _)| i).sum();

    (cpu, io, metrics, per_thread_data)
}
