use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::constants::*;
use crate::saturator::{CalibrationResult, TuningParams, create_shared_region, destroy_shared_region, worker_counters, now_ns, spawn_soi_gate_coordinator, spawn_victim_phase_coordinator, spawn_antiphase_gate_coordinator, bump_gate_epoch};
use crate::proc_metrics;
use crate::soi::SoiType;
use super::{aggregate_samples, TimeSeriesSample};

/// Spawn a single SoI worker process that uses the given shared memory region.
pub fn spawn_soi_worker(
    shm_name: &str,
    worker_id: usize,
    max_workers: usize,
    soi_type: SoiType,
    buffer_size: usize,
    nice: Option<i32>,
    active_phase: Option<u64>,
) -> std::process::Child {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new(&exe);
    cmd.arg("__soi_worker")
        .arg(shm_name)
        .arg(worker_id.to_string())
        .arg(max_workers.to_string())
        .arg(soi_type.name())
        .arg(buffer_size.to_string())
        .arg(match active_phase { Some(p) => p.to_string(), None => "none".to_string() });
    if let Some(n) = nice {
        unsafe {
            cmd.pre_exec(move || {
                let ret = libc::nice(n);
                if ret == -1 && *libc::__errno_location() != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.spawn().expect("Failed to spawn SoI worker process")
}

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
) -> (proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let worker_count = workers.len();
    let shm_name = format!("/saturator_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, worker_count);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let sleep_us = calibration.cpu_us as u64;
    let always_io = params.victim_phases.as_ref()
        .is_some_and(|phases| phases.iter().any(|&p| p > 0.0));
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
            .arg(always_io.to_string())
            .spawn()
            .expect("Failed to spawn worker process");
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    let victim_coord_handle = if let Some(period_ms) = params.victim_period_ms {
        let deadline_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).deadline_ns };
        let epoch_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).gate_epoch };
        if let Some(ref phases) = params.victim_phases {
            let io_perc_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).victim_io_perc };
            Some(spawn_victim_phase_coordinator(io_perc_ref, epoch_ref, deadline_ref, period_ms, phases.clone()))
        } else {
            let gate: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).victim_gate };
            Some(spawn_soi_gate_coordinator(gate, epoch_ref, deadline_ref, period_ms, 0.5))
        }
    } else {
        None
    };

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
    bump_gate_epoch(region);

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

            let (mut v_cpu_delta, mut v_io_delta) = (0u64, 0u64);
            for i in 0..worker_count {
                let cpu_d = curr_counters[i].0.saturating_sub(prev_counters[i].0);
                let io_d = curr_counters[i].1.saturating_sub(prev_counters[i].1);
                v_cpu_delta += cpu_d;
                v_io_delta += io_d;
            }

            let victim_on = unsafe { (*region_ptr).victim_gate.load(Ordering::Relaxed) } == 1;
            let io_perc_raw = unsafe { (*region_ptr).victim_io_perc.load(Ordering::Relaxed) };
            let victim_io_phase = if io_perc_raw == u64::MAX { -1.0 } else { io_perc_raw as f64 / 1000.0 };
            samples.push(TimeSeriesSample {
                elapsed_ms,
                victim_cpu_ops_sec: v_cpu_delta as f64 / interval_secs,
                victim_io_ops_sec: v_io_delta as f64 / interval_secs,
                soi_ops_sec: 0.0,
                soi_cpu_ops_sec: 0.0,
                soi_io_ops_sec: 0.0,
                soi_gate_on: false,
                victim_gate_on: victim_on,
                victim_io_phase,
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

    if let Some(h) = victim_coord_handle {
        let _ = h.join();
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

/// Run a single measurement with `worker_count` uniform processes.
pub fn measure_single_run_proc(
    worker_count: usize,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let workers: Vec<WorkerConfig> = (0..worker_count)
        .map(|_| WorkerConfig { io_perc, intensity: params.intensity })
        .collect();

    let (metrics, per_worker, timeseries) = run_proc_measurement(&workers, calibration, params);

    let (mut cpu, mut io) = (0.0, 0.0);
    for &(wc, wi, _, _) in &per_worker {
        cpu += wc;
        io += wi;
    }

    (cpu, io, metrics, per_worker, timeseries)
}

/// Collect multiple samples of process-based throughput and return aggregated results.
pub fn measure_proc_throughput(
    worker_count: usize,
    io_perc: f64,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Vec<Option<Vec<TimeSeriesSample>>>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_proc(worker_count, io_perc, calibration, params)
    }).collect();
    let all_timeseries: Vec<Option<Vec<TimeSeriesSample>>> = samples.iter().map(|s| s.4.clone()).collect();
    let base_samples: Vec<_> = samples.into_iter().map(|s| (s.0, s.1, s.2, s.3)).collect();
    let (cpu, io, cpu_sd, io_sd, metrics, pw) = aggregate_samples(base_samples);
    (cpu, io, cpu_sd, io_sd, metrics, pw, all_timeseries)
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

    let (metrics, per_worker, _timeseries) = run_proc_measurement(&workers, calibration, params);

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

    let (metrics, per_worker, _timeseries) = run_proc_measurement(&workers, calibration, params);

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
    Standard(WorkerConfig),
    Soi { soi_type: SoiType, buffer_size: usize, active_phase: Option<u64> },
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
    let always_io = params.victim_phases.as_ref()
        .is_some_and(|phases| phases.iter().any(|&p| p > 0.0));
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
                    .arg(always_io.to_string())
                    .spawn()
                    .expect("Failed to spawn worker process")
            }
            WorkerKind::Soi { soi_type, buffer_size, active_phase } => {
                spawn_soi_worker(&shm_name, i, worker_count, *soi_type, *buffer_size, params.nice, *active_phase)
            }
        };
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    let (soi_gate_handle, victim_coord_handle) = if params.antiphase {
        let victim_gate: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).victim_gate };
        let soi_gate: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).soi_gate };
        let epoch_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).gate_epoch };
        let deadline_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).deadline_ns };
        let period_ms = params.victim_period_ms.unwrap();
        let h = spawn_antiphase_gate_coordinator(victim_gate, soi_gate, epoch_ref, deadline_ref, period_ms, params.soi_duty);
        (None, Some(h))
    } else {
        let soi_h = if let Some(period_ms) = params.soi_period_ms {
            let gate: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).soi_gate };
            let epoch_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).gate_epoch };
            let deadline_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).deadline_ns };
            Some(spawn_soi_gate_coordinator(gate, epoch_ref, deadline_ref, period_ms, params.soi_duty))
        } else {
            None
        };
        let victim_h = if let Some(period_ms) = params.victim_period_ms {
            let deadline_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).deadline_ns };
            let epoch_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).gate_epoch };
            if let Some(ref phases) = params.victim_phases {
                let io_perc_ref: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).victim_io_perc };
                Some(spawn_victim_phase_coordinator(io_perc_ref, epoch_ref, deadline_ref, period_ms, phases.clone()))
            } else {
                let gate: &'static std::sync::atomic::AtomicU64 = unsafe { &(*region_ptr).victim_gate };
                Some(spawn_soi_gate_coordinator(gate, epoch_ref, deadline_ref, period_ms, 0.5))
            }
        } else {
            None
        };
        (soi_h, victim_h)
    };

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
    bump_gate_epoch(region);

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
            let (mut v_cpu_delta, mut v_io_delta) = (0u64, 0u64);
            let (mut soi_cpu_delta, mut soi_io_delta) = (0u64, 0u64);
            for i in 0..worker_count {
                let cpu_d = curr_counters[i].0.saturating_sub(prev_counters[i].0);
                let io_d = curr_counters[i].1.saturating_sub(prev_counters[i].1);
                if i < victim_count {
                    v_cpu_delta += cpu_d;
                    v_io_delta += io_d;
                } else {
                    soi_cpu_delta += cpu_d;
                    soi_io_delta += io_d;
                }
            }

            let soi_on = unsafe { (*region_ptr).soi_gate.load(Ordering::Relaxed) } == 1;
            let victim_on = unsafe { (*region_ptr).victim_gate.load(Ordering::Relaxed) } == 1;
            let io_perc_raw = unsafe { (*region_ptr).victim_io_perc.load(Ordering::Relaxed) };
            let victim_io_phase = if io_perc_raw == u64::MAX { -1.0 } else { io_perc_raw as f64 / 1000.0 };
            samples.push(TimeSeriesSample {
                elapsed_ms,
                victim_cpu_ops_sec: v_cpu_delta as f64 / interval_secs,
                victim_io_ops_sec: v_io_delta as f64 / interval_secs,
                soi_ops_sec: (soi_cpu_delta + soi_io_delta) as f64 / interval_secs,
                soi_cpu_ops_sec: soi_cpu_delta as f64 / interval_secs,
                soi_io_ops_sec: soi_io_delta as f64 / interval_secs,
                soi_gate_on: soi_on,
                victim_gate_on: victim_on,
                victim_io_phase,
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

    if let Some(h) = soi_gate_handle {
        let _ = h.join();
    }
    if let Some(h) = victim_coord_handle {
        let _ = h.join();
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
/// Returns (victim_cpu, victim_io, soi_ops, soi_cpu_ops, soi_io_ops, metrics, per_worker, timeseries).
pub fn measure_single_run_soi(
    victim_count: usize,
    victim_io_perc: f64,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let mut workers: Vec<WorkerKind> = (0..victim_count)
        .map(|_| WorkerKind::Standard(WorkerConfig { io_perc: victim_io_perc, intensity: params.intensity }))
        .collect();
    for _ in 0..soi_count {
        workers.push(WorkerKind::Soi { soi_type, buffer_size: soi_buffer_size, active_phase: None });
    }

    let (metrics, per_worker, timeseries) = run_mixed_proc_measurement(&workers, victim_count, calibration, params);

    // Split results: first victim_count are victims, rest are SoI
    let (mut victim_cpu, mut victim_io) = (0.0, 0.0);
    let (mut soi_cpu_ops, mut soi_io_ops) = (0.0, 0.0);
    for (i, &(wc, wi, _, _)) in per_worker.iter().enumerate() {
        if i < victim_count {
            victim_cpu += wc;
            victim_io += wi;
        } else {
            soi_cpu_ops += wc;
            soi_io_ops += wi;
        }
    }

    (victim_cpu, victim_io, soi_cpu_ops + soi_io_ops, soi_cpu_ops, soi_io_ops, metrics, per_worker, timeseries)
}

/// Multi-sample SoI measurement. Returns:
/// (victim_cpu, victim_io, victim_cpu_stddev, victim_io_stddev, soi_ops, soi_cpu_ops, soi_io_ops, metrics, per_worker, timeseries)
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
) -> (f64, f64, f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Vec<Option<Vec<TimeSeriesSample>>>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_soi(
            victim_count, victim_io_perc, soi_count, soi_type, soi_buffer_size,
            calibration, params,
        )
    }).collect();

    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();
    let soi_vals: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let soi_cpu_vals: Vec<f64> = samples.iter().map(|s| s.3).collect();
    let soi_io_vals: Vec<f64> = samples.iter().map(|s| s.4).collect();
    let metrics_list: Vec<_> = samples.iter().map(|s| s.5.clone()).collect();

    let median_total = super::median(&cpu_vals) + super::median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].6.clone();
    let all_timeseries: Vec<Option<Vec<TimeSeriesSample>>> = samples.into_iter().map(|s| s.7).collect();

    (
        super::median(&cpu_vals),
        super::median(&io_vals),
        super::stddev(&cpu_vals),
        super::stddev(&io_vals),
        super::median(&soi_vals),
        super::median(&soi_cpu_vals),
        super::median(&soi_io_vals),
        proc_metrics::median_metrics(&metrics_list),
        per_worker,
        all_timeseries,
    )
}

/// Single run with multiple SoI groups, each optionally phase-gated.
/// `soi_groups`: (soi_type, buffer_size, count, active_phase_encoded)
pub fn measure_single_run_soi_phased(
    victim_count: usize,
    victim_io_perc: f64,
    soi_groups: &[(SoiType, usize, usize, Option<u64>)],
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<TimeSeriesSample>>) {
    let mut workers: Vec<WorkerKind> = (0..victim_count)
        .map(|_| WorkerKind::Standard(WorkerConfig { io_perc: victim_io_perc, intensity: params.intensity }))
        .collect();
    for &(soi_type, buffer_size, count, active_phase) in soi_groups {
        for _ in 0..count {
            workers.push(WorkerKind::Soi { soi_type, buffer_size, active_phase });
        }
    }

    let (metrics, per_worker, timeseries) = run_mixed_proc_measurement(&workers, victim_count, calibration, params);

    let (mut victim_cpu, mut victim_io) = (0.0, 0.0);
    let (mut soi_cpu_ops, mut soi_io_ops) = (0.0, 0.0);
    for (i, &(wc, wi, _, _)) in per_worker.iter().enumerate() {
        if i < victim_count {
            victim_cpu += wc;
            victim_io += wi;
        } else {
            soi_cpu_ops += wc;
            soi_io_ops += wi;
        }
    }

    (victim_cpu, victim_io, soi_cpu_ops + soi_io_ops, soi_cpu_ops, soi_io_ops, metrics, per_worker, timeseries)
}

/// Multi-sample phased SoI measurement.
pub fn measure_soi_phased_throughput(
    victim_count: usize,
    victim_io_perc: f64,
    soi_groups: &[(SoiType, usize, usize, Option<u64>)],
    calibration: &CalibrationResult,
    params: &TuningParams,
) -> (f64, f64, f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Vec<Option<Vec<TimeSeriesSample>>>) {
    let samples: Vec<_> = (0..params.samples).map(|_| {
        measure_single_run_soi_phased(
            victim_count, victim_io_perc, soi_groups,
            calibration, params,
        )
    }).collect();

    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();
    let soi_vals: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let soi_cpu_vals: Vec<f64> = samples.iter().map(|s| s.3).collect();
    let soi_io_vals: Vec<f64> = samples.iter().map(|s| s.4).collect();
    let metrics_list: Vec<_> = samples.iter().map(|s| s.5.clone()).collect();

    let median_total = super::median(&cpu_vals) + super::median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].6.clone();
    let all_timeseries: Vec<Option<Vec<TimeSeriesSample>>> = samples.into_iter().map(|s| s.7).collect();

    (
        super::median(&cpu_vals),
        super::median(&io_vals),
        super::stddev(&cpu_vals),
        super::stddev(&io_vals),
        super::median(&soi_vals),
        super::median(&soi_cpu_vals),
        super::median(&soi_io_vals),
        proc_metrics::median_metrics(&metrics_list),
        per_worker,
        all_timeseries,
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
