use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::saturator::{TuningParams, create_shared_region, destroy_shared_region, worker_counters, now_ns};
use crate::proc_metrics;
use crate::soi::SoiType;
use super::ext_throughput::ThroughputReader;
use super::spawn_soi_worker;

/// Result of a single external workload measurement run.
pub struct ExtMeasurementResult {
    pub ext_ops_per_sec: f64,
    pub soi_ops: f64,
    pub metrics: proc_metrics::SystemMetrics,
    pub soi_per_worker: Vec<(f64, f64, f64, u64)>,
    pub timeseries: Option<Vec<ExtTimeSeriesSample>>,
}

/// A single time-series sample for external workload experiments.
pub struct ExtTimeSeriesSample {
    pub elapsed_ms: u64,
    pub ext_ops_sec: f64,
    pub soi_ops_sec: f64,
    pub metrics: proc_metrics::SystemMetrics,
}

/// Run a single measurement: external workload command + SoI workers.
///
/// Lifecycle:
/// 1. Delete stale throughput file
/// 2. Launch external workload via shell
/// 3. If soi_count > 0: create shared memory, spawn SoI workers, wait for ready
/// 4. Warmup period
/// 5. Reset throughput reader, zero SoI counters, take cgroup snapshot
/// 6. Set SoI deadline, run sampling loop
/// 7. Take final cgroup snapshot
/// 8. Kill external workload, wait for SoI workers
/// 9. Collect results
pub fn run_ext_measurement(
    ext_cmd: &str,
    throughput_file: &str,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    params: &TuningParams,
) -> ExtMeasurementResult {
    // Clean up stale throughput file
    let _ = std::fs::remove_file(throughput_file);

    // Launch external workload
    let mut ext_child = Command::new("sh")
        .arg("-c")
        .arg(ext_cmd)
        .env("SATURATOR_THROUGHPUT_FILE", throughput_file)
        .spawn()
        .expect("Failed to spawn external workload");

    // Set up SoI workers (if any)
    let soi_state = if soi_count > 0 {
        let shm_name = format!("/saturator_ext_{}_{}", std::process::id(), soi_count);
        let (region_ptr, shm_fd) = create_shared_region(&shm_name, soi_count);
        let region = unsafe { &*region_ptr };

        let mut children = Vec::with_capacity(soi_count);
        for i in 0..soi_count {
            children.push(spawn_soi_worker(&shm_name, i, soi_count, soi_type, soi_buffer_size));
        }

        // Wait for all SoI workers to signal ready
        while region.ready_count.load(Ordering::Relaxed) < soi_count as u64 {
            std::thread::sleep(Duration::from_millis(10));
        }

        Some((shm_name, region_ptr, shm_fd, children))
    } else {
        None
    };

    // Warmup
    std::thread::sleep(Duration::from_secs(params.warmup_secs));

    // Reset: throughput reader skips warmup data, zero SoI counters
    let mut tp_reader = ThroughputReader::new(throughput_file);
    tp_reader.reset();

    if let Some((_, region_ptr, _, _)) = &soi_state {
        for i in 0..soi_count {
            let (wk_cpu, wk_io, wk_sleep, _) = unsafe { worker_counters(*region_ptr, i) };
            wk_cpu.store(0, Ordering::Relaxed);
            wk_io.store(0, Ordering::Relaxed);
            wk_sleep.store(0, Ordering::Relaxed);
        }
    }

    // Start measurement
    let snap_before = proc_metrics::take_snapshot();

    if let Some((_, region_ptr, _, _)) = &soi_state {
        let region = unsafe { &**region_ptr };
        let deadline = now_ns() + params.duration_secs * 1_000_000_000;
        region.deadline_ns.store(deadline, Ordering::Relaxed);
    }

    // Sampling loop or single sleep
    let mut all_tp_samples: Vec<(u64, f64)> = Vec::new();
    let timeseries = if let Some(interval_ms) = params.sample_interval_ms {
        let interval_secs = interval_ms as f64 / 1000.0;
        let duration_ms = params.duration_secs * 1000;
        let mut ts_samples = Vec::new();
        let mut prev_soi_counters = read_soi_counters(&soi_state, soi_count);
        let mut prev_snap = snap_before.clone();
        let mut elapsed_ms: u64 = 0;
        let mut last_ext_ops = 0.0_f64;

        loop {
            std::thread::sleep(Duration::from_millis(interval_ms));
            elapsed_ms += interval_ms;

            // Read external throughput (rates computed from cumulative deltas)
            let new_tp = tp_reader.read_new_samples();
            if !new_tp.is_empty() {
                last_ext_ops = ThroughputReader::average_ops(&new_tp);
            }
            all_tp_samples.extend_from_slice(&new_tp);

            // Read SoI counters
            let curr_soi_counters = read_soi_counters(&soi_state, soi_count);
            let mut soi_delta = 0u64;
            for i in 0..soi_count {
                soi_delta += curr_soi_counters[i].0.saturating_sub(prev_soi_counters[i].0);
                soi_delta += curr_soi_counters[i].1.saturating_sub(prev_soi_counters[i].1);
            }

            let curr_snap = proc_metrics::take_snapshot();
            let delta_metrics = proc_metrics::compute_delta(&prev_snap, &curr_snap, interval_secs);

            ts_samples.push(ExtTimeSeriesSample {
                elapsed_ms,
                ext_ops_sec: last_ext_ops,
                soi_ops_sec: soi_delta as f64 / interval_secs,
                metrics: delta_metrics,
            });

            prev_soi_counters = curr_soi_counters;
            prev_snap = curr_snap;

            if elapsed_ms >= duration_ms { break; }
        }
        Some(ts_samples)
    } else {
        std::thread::sleep(Duration::from_secs(params.duration_secs));
        // Read all throughput samples accumulated during measurement
        all_tp_samples = tp_reader.read_new_samples();
        None
    };

    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, params.duration_secs as f64);

    // Terminate external workload: SIGTERM, then SIGKILL after 2s
    let ext_pid = ext_child.id();
    unsafe { libc::kill(ext_pid as i32, libc::SIGTERM); }
    match ext_child.try_wait() {
        Ok(Some(_)) => {}
        _ => {
            std::thread::sleep(Duration::from_secs(2));
            let _ = ext_child.kill();
            let _ = ext_child.wait();
        }
    }

    // Collect SoI results
    let mut soi_per_worker = Vec::new();
    let mut soi_ops_total = 0.0;
    if let Some((shm_name, region_ptr, shm_fd, mut children)) = soi_state {
        for mut child in children.drain(..) {
            let _ = child.wait();
        }
        for i in 0..soi_count {
            let (wk_cpu, wk_io, wk_sleep, wk_errors) = unsafe { worker_counters(region_ptr, i) };
            let wc = wk_cpu.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
            let wi = wk_io.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
            let ws = wk_sleep.load(Ordering::Relaxed) as f64 / params.duration_secs as f64;
            let we = wk_errors.load(Ordering::Relaxed);
            soi_ops_total += wc + wi;
            soi_per_worker.push((wc, wi, ws, we));
        }
        destroy_shared_region(&shm_name, region_ptr, shm_fd, soi_count);
    }

    let ext_ops_per_sec = ThroughputReader::average_ops(&all_tp_samples);

    ExtMeasurementResult {
        ext_ops_per_sec,
        soi_ops: soi_ops_total,
        metrics,
        soi_per_worker,
        timeseries,
    }
}

/// Run multiple measurement samples and return aggregated results.
/// Returns: (median_ext_ops, ext_stddev, median_soi_ops, median_metrics, soi_per_worker, timeseries)
pub fn measure_ext_throughput(
    ext_cmd: &str,
    throughput_file: &str,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    params: &TuningParams,
) -> (f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>, Option<Vec<ExtTimeSeriesSample>>) {
    let results: Vec<ExtMeasurementResult> = (0..params.samples).map(|_| {
        run_ext_measurement(ext_cmd, throughput_file, soi_count, soi_type, soi_buffer_size, params)
    }).collect();

    let ext_vals: Vec<f64> = results.iter().map(|r| r.ext_ops_per_sec).collect();
    let soi_vals: Vec<f64> = results.iter().map(|r| r.soi_ops).collect();
    let metrics_list: Vec<_> = results.iter().map(|r| r.metrics.clone()).collect();

    let median_ext = super::median(&ext_vals);
    let ext_stddev = super::stddev(&ext_vals);
    let median_soi = super::median(&soi_vals);

    // Select per-worker data and timeseries from the sample closest to median
    let median_idx = results.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.ext_ops_per_sec - median_ext).abs()
                .partial_cmp(&(b.ext_ops_per_sec - median_ext).abs()).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    let soi_per_worker = results[median_idx].soi_per_worker.clone();
    let timeseries = results.into_iter().nth(median_idx).and_then(|r| r.timeseries);

    (median_ext, ext_stddev, median_soi, proc_metrics::median_metrics(&metrics_list), soi_per_worker, timeseries)
}

/// Read SoI worker counters from shared memory (if SoI workers are active).
fn read_soi_counters(
    soi_state: &Option<(String, *mut crate::saturator::SharedRegion, i32, Vec<std::process::Child>)>,
    soi_count: usize,
) -> Vec<(u64, u64)> {
    if let Some((_, region_ptr, _, _)) = soi_state {
        (0..soi_count).map(|i| {
            let (wk_cpu, wk_io, _, _) = unsafe { worker_counters(*region_ptr, i) };
            (wk_cpu.load(Ordering::Relaxed), wk_io.load(Ordering::Relaxed))
        }).collect()
    } else {
        Vec::new()
    }
}
