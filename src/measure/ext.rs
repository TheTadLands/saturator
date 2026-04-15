use std::os::unix::process::CommandExt;
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
    /// Total ops completed by the external workload during the measurement
    /// window (cumulative_end - cumulative_start from the throughput protocol).
    pub total_ops: f64,
    /// Elapsed seconds spanned by total_ops (may differ slightly from
    /// params.duration_secs due to sampling alignment).
    pub total_ops_secs: f64,
    /// Raw per-second rate samples over the measurement window. Retained so
    /// aggregation across samples can pool rates before percentiling instead of
    /// collapsing each run to its own percentile first.
    pub rate_samples: Vec<f64>,
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

    // Launch external workload in its own process group so we can signal the
    // whole group (including any backgrounded children like db_bench) at teardown.
    // sh does not propagate signals to backgrounded children by default, so
    // without a group-wide signal db_bench survives wrapper teardown, reparents
    // to init, and corrupts the shared throughput file across subsequent samples.
    let mut ext_child = Command::new("sh")
        .arg("-c")
        .arg(ext_cmd)
        .env("SATURATOR_THROUGHPUT_FILE", throughput_file)
        .process_group(0)
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

    // Terminate external workload: SIGTERM the whole process group, then SIGKILL
    // the group after 2s. Negative PID targets the group (kill(2)).
    let ext_pid = ext_child.id() as i32;
    unsafe { libc::kill(-ext_pid, libc::SIGTERM); }
    match ext_child.try_wait() {
        Ok(Some(_)) => {}
        _ => {
            std::thread::sleep(Duration::from_secs(2));
            unsafe { libc::kill(-ext_pid, libc::SIGKILL); }
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
    let (total_ops, total_ops_secs) = tp_reader.total_ops().unwrap_or((0.0, 0.0));
    let rate_samples: Vec<f64> = all_tp_samples.iter().map(|(_, r)| *r).collect();

    ExtMeasurementResult {
        ext_ops_per_sec,
        total_ops,
        total_ops_secs,
        rate_samples,
        soi_ops: soi_ops_total,
        metrics,
        soi_per_worker,
        timeseries,
    }
}

/// Aggregated result of multiple measurement samples for an external-workload run.
pub struct ExtAggregateResult {
    pub median_ext_ops: f64,
    pub ext_stddev: f64,
    /// Sum of total_ops across samples and the total elapsed seconds they
    /// cover. `total_ops / total_secs` gives the pooled mean rate, which for a
    /// bimodal victim is a more defensible summary than the median of rates.
    pub total_ops: f64,
    pub total_ops_secs: f64,
    /// Percentiles computed over per-second rates pooled across all samples.
    pub p10_ops_sec: f64,
    pub p50_ops_sec: f64,
    pub p90_ops_sec: f64,
    pub median_soi_ops: f64,
    pub metrics: proc_metrics::SystemMetrics,
    pub soi_per_worker: Vec<(f64, f64, f64, u64)>,
    pub timeseries: Option<Vec<ExtTimeSeriesSample>>,
    pub per_sample_ext_ops: Vec<f64>,
    pub per_sample_soi_ops: Vec<f64>,
}

/// Run multiple measurement samples and return aggregated results plus the
/// raw per-sample `(ext_ops, soi_ops)` values (in sample order) so callers can
/// emit long-format per-sample CSVs.
pub fn measure_ext_throughput(
    ext_cmd: &str,
    throughput_file: &str,
    soi_count: usize,
    soi_type: SoiType,
    soi_buffer_size: usize,
    params: &TuningParams,
) -> ExtAggregateResult {
    let results: Vec<ExtMeasurementResult> = (0..params.samples).enumerate().map(|(i, _)| {
        // Idle gap between samples so post-kill compaction drains and the
        // system reaches a consistent starting state before the next warmup.
        if i > 0 && params.cooldown_secs > 0 {
            std::thread::sleep(Duration::from_secs(params.cooldown_secs));
        }
        run_ext_measurement(ext_cmd, throughput_file, soi_count, soi_type, soi_buffer_size, params)
    }).collect();

    let ext_vals: Vec<f64> = results.iter().map(|r| r.ext_ops_per_sec).collect();
    let soi_vals: Vec<f64> = results.iter().map(|r| r.soi_ops).collect();
    let metrics_list: Vec<_> = results.iter().map(|r| r.metrics.clone()).collect();

    let median_ext = super::median(&ext_vals);
    let ext_stddev = super::stddev(&ext_vals);
    let median_soi = super::median(&soi_vals);

    let total_ops: f64 = results.iter().map(|r| r.total_ops).sum();
    let total_ops_secs: f64 = results.iter().map(|r| r.total_ops_secs).sum();
    let pooled: Vec<(u64, f64)> = results.iter()
        .flat_map(|r| r.rate_samples.iter().map(|v| (0u64, *v)))
        .collect();
    let pcts = super::ext_throughput::ThroughputReader::percentiles(&pooled, &[0.10, 0.50, 0.90]);
    let (p10, p50, p90) = (pcts[0], pcts[1], pcts[2]);

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

    ExtAggregateResult {
        median_ext_ops: median_ext,
        ext_stddev,
        total_ops,
        total_ops_secs,
        p10_ops_sec: p10,
        p50_ops_sec: p50,
        p90_ops_sec: p90,
        median_soi_ops: median_soi,
        metrics: proc_metrics::median_metrics(&metrics_list),
        soi_per_worker,
        timeseries,
        per_sample_ext_ops: ext_vals,
        per_sample_soi_ops: soi_vals,
    }
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
