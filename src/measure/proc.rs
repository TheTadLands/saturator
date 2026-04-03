use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::constants::*;
use crate::saturator::{CalibrationResult, TuningParams, create_shared_region, destroy_shared_region, worker_counters, now_ns};
use crate::proc_metrics;
use crate::soi::SoiType;
use super::{aggregate_samples, TimeSeriesSample};

/// Per-worker configuration for a process-based measurement run.
pub struct WorkerConfig {
    pub io_perc: f64,
    pub intensity: f64,
}

/// Core lifecycle for process-based measurements: create shm, spawn children, warmup,
/// measure, reap, collect per-worker counters, destroy shm.
///
/// Returns (SystemMetrics, per_worker_rates) where per_worker_rates is a Vec of
/// (cpu_ops/s, io_ops/s, sleep_ops/s) per worker in spawn order.
fn run_proc_measurement(
    workers: &[WorkerConfig],
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let worker_count = workers.len();
    let shm_name = format!("/saturator_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, worker_count);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let sleep_us = calibration.cpu_us as u64;
    let mut children = Vec::with_capacity(worker_count);

    for (i, wc) in workers.iter().enumerate() {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(calibration.cpu_iterations.to_string())
            .arg(calibration.io_iterations.to_string())
            .arg(wc.io_perc.to_string())
            .arg(params.buffer_kb.to_string())
            .arg(params.io_buffer_kb.to_string())
            .arg(wc.intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(worker_count.to_string())
            .arg(params.random_access.to_string())
            .arg(params.direct_io.to_string())
            .spawn()
            .expect("Failed to spawn worker process");
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(params.warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep, _wk_errors) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
        // Don't reset errors — they're cumulative diagnostics
    }

    let snap_before = proc_metrics::take_snapshot();
    let deadline = now_ns() + params.duration_secs * 1_000_000_000;
    region.deadline_ns.store(deadline, Ordering::Relaxed);
    std::thread::sleep(Duration::from_secs(params.duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, params.duration_secs as f64);

    for mut child in children {
        let _ = child.wait();
    }

    // Collect per-worker rates
    let mut per_worker = Vec::with_capacity(worker_count);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep, wk_errors) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let we = wk_errors.load(Ordering::Relaxed);
        per_worker.push((wc, wi, ws, we));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, worker_count);

    (metrics, per_worker)
}

/// Run a single measurement with `worker_count` uniform processes.
pub fn measure_single_run_proc(
    worker_count: usize,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let workers: Vec<WorkerConfig> = (0..worker_count)
        .map(|_| WorkerConfig { io_perc, intensity: params.intensity })
        .collect();

    let (metrics, per_worker) = run_proc_measurement(&workers, calibration, params);

    let (mut cpu, mut io) = (0.0, 0.0);
    for &(wc, wi, _, _) in &per_worker {
        cpu += wc;
        io += wi;
    }

    (cpu, io, metrics, per_worker)
}

/// Collect multiple samples of process-based throughput and return aggregated results.
pub fn measure_proc_throughput(
    worker_count: usize,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_proc(worker_count, io_perc, calibration, params)
    }).collect();
    aggregate_samples(samples)
}

/// Run a single measurement with base workers at intensity=1.0 and one probe worker at `probe_intensity`.
pub fn measure_single_run_proc_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let mut workers: Vec<WorkerConfig> = (0..base_workers)
        .map(|_| WorkerConfig { io_perc, intensity: 1.0 })
        .collect();
    workers.push(WorkerConfig { io_perc, intensity: probe_intensity });

    let (metrics, per_worker) = run_proc_measurement(&workers, calibration, params);

    let (mut cpu, mut io) = (0.0, 0.0);
    for &(wc, wi, _, _) in &per_worker {
        cpu += wc;
        io += wi;
    }

    (cpu, io, metrics, per_worker)
}

/// Collect multiple samples of mixed-intensity process throughput and return aggregated results.
pub fn measure_proc_throughput_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_proc_mixed_intensity(
            base_workers, probe_intensity, io_perc, calibration, params,
        )
    }).collect();
    aggregate_samples(samples)
}

/// Run a single measurement of baseline+extra workers with different I/O ratios (process mode).
pub fn measure_single_run_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let mut workers: Vec<WorkerConfig> = (0..baseline_workers)
        .map(|_| WorkerConfig { io_perc: baseline_io_perc, intensity: params.intensity })
        .collect();
    for _ in 0..extra_workers {
        workers.push(WorkerConfig { io_perc: extra_io_perc, intensity: params.intensity });
    }

    let (metrics, per_worker) = run_proc_measurement(&workers, calibration, params);

    let (mut baseline_cpu, mut baseline_io) = (0.0, 0.0);
    let (mut extra_cpu, mut extra_io) = (0.0, 0.0);
    for (i, &(wc, wi, _, _)) in per_worker.iter().enumerate() {
        if i < baseline_workers {
            baseline_cpu += wc;
            baseline_io += wi;
        } else {
            extra_cpu += wc;
            extra_io += wi;
        }
    }

    (baseline_cpu, baseline_io, extra_cpu, extra_io, metrics, per_worker)
}

/// What kind of worker to spawn in a mixed measurement.
pub enum WorkerKind {
    /// Standard saturator worker with calibrated workload.
    Standard(WorkerConfig),
    /// SoI interference worker — no calibration, dedicated work function.
    Soi { soi_type: SoiType, buffer_size: usize },
}

/// Core lifecycle for mixed measurements: victims + SoI workers sharing the same shm region.
/// Returns (SystemMetrics, per_worker_rates, optional time-series) where per_worker_rates is a Vec of
/// (cpu_ops/s, io_ops/s, sleep_ops/s, errors) per worker in spawn order.
///
/// `victim_count` is the number of leading workers that are victims (used for time-series splitting).
fn run_mixed_proc_measurement(
    workers: &[WorkerKind],
    victim_count: usize,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let worker_count = workers.len();
    let shm_name = format!("/saturator_soi_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, worker_count);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let sleep_us = calibration.cpu_us as u64;
    let mut children = Vec::with_capacity(worker_count);

    for (i, wk) in workers.iter().enumerate() {
        let child = match wk {
            WorkerKind::Standard(wc) => {
                Command::new(&exe)
                    .arg("__worker")
                    .arg(&shm_name)
                    .arg(i.to_string())
                    .arg(calibration.cpu_iterations.to_string())
                    .arg(calibration.io_iterations.to_string())
                    .arg(wc.io_perc.to_string())
                    .arg(params.buffer_kb.to_string())
                    .arg(params.io_buffer_kb.to_string())
                    .arg(wc.intensity.to_string())
                    .arg(sleep_us.to_string())
                    .arg(worker_count.to_string())
                    .arg(params.random_access.to_string())
                    .arg(params.direct_io.to_string())
                    .spawn()
                    .expect("Failed to spawn worker process")
            }
            WorkerKind::Soi { soi_type, buffer_size } => {
                Command::new(&exe)
                    .arg("__soi_worker")
                    .arg(&shm_name)
                    .arg(i.to_string())
                    .arg(worker_count.to_string())
                    .arg(soi_type.name())
                    .arg(buffer_size.to_string())
                    .spawn()
                    .expect("Failed to spawn SoI worker process")
            }
        };
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(params.warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep, _wk_errors) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    let deadline = now_ns() + params.duration_secs * 1_000_000_000;
    region.deadline_ns.store(deadline, Ordering::Relaxed);

    // Time-series sampling loop or single sleep
    let timeseries = if let Some(interval_ms) = params.sample_interval_ms {
        let interval_secs = interval_ms as f64 / 1000.0;
        let duration_ms = params.duration_secs * 1000;
        let mut samples = Vec::new();
        let mut prev_counters = read_all_worker_counters(region_ptr, worker_count);
        let mut prev_snap = snap_before.clone();
        let mut elapsed_ms: u64 = 0;

        loop {
            std::thread::sleep(Duration::from_millis(interval_ms));
            elapsed_ms += interval_ms;

            let curr_counters = read_all_worker_counters(region_ptr, worker_count);
            let curr_snap = proc_metrics::take_snapshot();
            let delta_metrics = proc_metrics::compute_delta(&prev_snap, &curr_snap, interval_secs);

            // Sum deltas split by victim vs SoI
            let (mut v_cpu_delta, mut v_io_delta, mut soi_delta) = (0u64, 0u64, 0u64);
            for i in 0..worker_count {
                let cpu_d = curr_counters[i].0.saturating_sub(prev_counters[i].0);
                let io_d = curr_counters[i].1.saturating_sub(prev_counters[i].1);
                if i < victim_count {
                    v_cpu_delta += cpu_d;
                    v_io_delta += io_d;
                } else {
                    soi_delta += cpu_d + io_d;
                }
            }

            samples.push(TimeSeriesSample {
                elapsed_ms,
                victim_cpu_ops_sec: v_cpu_delta as f64 / interval_secs,
                victim_io_ops_sec: v_io_delta as f64 / interval_secs,
                soi_ops_sec: soi_delta as f64 / interval_secs,
                metrics: delta_metrics,
            });

            prev_counters = curr_counters;
            prev_snap = curr_snap;

            if elapsed_ms >= duration_ms { break; }
        }
        Some(samples)
    } else {
        std::thread::sleep(Duration::from_secs(params.duration_secs));
        None
    };

    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, params.duration_secs as f64);

    for mut child in children {
        let _ = child.wait();
    }

    // Collect per-worker rates
    let mut per_worker = Vec::with_capacity(worker_count);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep, wk_errors) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
        let we = wk_errors.load(Ordering::Relaxed);
        per_worker.push((wc, wi, ws, we));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, worker_count);

    (metrics, per_worker, timeseries)
}

/// Read all per-worker (cpu_ops, io_ops) counters from shared memory.
/// Safe to call while workers are running — values may be up to BATCH_FLUSH_THRESHOLD ops stale.
fn read_all_worker_counters(
    region_ptr: *mut crate::saturator::SharedRegion,
    worker_count: usize,
) -> Vec<(u64, u64)> {
    (0..worker_count).map(|i| {
        let (wk_cpu, wk_io, _, _) = unsafe { worker_counters(region_ptr, i) };
        (wk_cpu.load(Ordering::Relaxed), wk_io.load(Ordering::Relaxed))
    }).collect()
}

/// Run a single SoI measurement: `victim_count` victim workers + `soi_count` SoI workers.
/// Returns (victim_cpu, victim_io, soi_ops, metrics, per_worker, timeseries).
pub fn measure_single_run_soi(
    victim_count: usize,
    victim_io_perc: f64,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let mut workers: Vec<WorkerKind> = (0..victim_count)
        .map(|_| WorkerKind::Standard(WorkerConfig { io_perc: victim_io_perc, intensity: params.intensity }))
        .collect();
    for _ in 0..soi_count {
        workers.push(WorkerKind::Soi { soi_type, buffer_size: soi_buffer_size });
    }

    let (metrics, per_worker, timeseries) = run_mixed_proc_measurement(&workers, victim_count, calibration, params);

    // Split results: first victim_count are victims, rest are SoI
    let (mut victim_cpu, mut victim_io) = (0.0, 0.0);
    let mut soi_ops = 0.0;
    for (i, &(wc, wi, _, _)) in per_worker.iter().enumerate() {
        if i < victim_count {
            victim_cpu += wc;
            victim_io += wi;
        } else {
            soi_ops += wc + wi;
        }
    }

    (victim_cpu, victim_io, soi_ops, metrics, per_worker, timeseries)
}

/// Multi-sample SoI measurement. Returns:
/// (victim_cpu, victim_io, victim_cpu_stddev, victim_io_stddev, soi_ops, metrics, per_worker, timeseries)
///
/// Time-series data is taken from the sample closest to the median (same as per_worker selection).
pub fn measure_soi_throughput(
    victim_count: usize,
    victim_io_perc: f64,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_soi(
            victim_count, victim_io_perc, soi_count, soi_type, soi_buffer_size,
            calibration, params,
        )
    }).collect();

    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();
    let soi_vals: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let metrics_list: Vec<_> = samples.iter().map(|s| s.3.clone()).collect();

    let median_total = super::median(&cpu_vals) + super::median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].4.clone();
    let timeseries = samples.into_iter().nth(median_idx).and_then(|s| s.5);

    (
        super::median(&cpu_vals),
        super::median(&io_vals),
        super::stddev(&cpu_vals),
        super::stddev(&io_vals),
        super::median(&soi_vals),
        proc_metrics::median_metrics(&metrics_list),
        per_worker,
        timeseries,
    )
}

/// Collect multiple samples of process-based slack measurement and return aggregated results.
pub fn measure_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_proc_slack(
            baseline_workers, extra_workers, baseline_io_perc, extra_io_perc,
            calibration, params,
        )
    }).collect();

    let b_cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let b_io_vals: Vec<f64>  = samples.iter().map(|s| s.1).collect();
    let e_cpu_vals: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let e_io_vals: Vec<f64>  = samples.iter().map(|s| s.3).collect();
    let metrics_list: Vec<_> = samples.iter().map(|s| s.4.clone()).collect();

    let median_baseline = super::median(&b_cpu_vals) + super::median(&b_io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            ((a.0 + a.1) - median_baseline).abs()
                .partial_cmp(&((b.0 + b.1) - median_baseline).abs()).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].5.clone();

    (
        super::median(&b_cpu_vals), super::median(&b_io_vals),
        super::median(&e_cpu_vals), super::median(&e_io_vals),
        super::stddev(&b_cpu_vals), super::stddev(&b_io_vals),
        proc_metrics::median_metrics(&metrics_list),
        per_worker,
    )
}
