use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::constants::*;
use crate::saturator::{create_shared_region, destroy_shared_region, worker_counters};
use crate::proc_metrics;
use super::aggregate_samples;

/// Collect multiple samples of process-based throughput and return aggregated results.
pub fn measure_proc_throughput(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<_> = (0..num_samples).map(|_| {
        measure_single_run_proc(worker_count, cpu_iterations, io_iterations, duration_secs, io_perc, buffer_kb, io_buffer_kb, intensity, sleep_us, warmup_secs, random_access, direct_io)
    }).collect();
    aggregate_samples(samples)
}

/// Run a single measurement with `worker_count` child processes.
pub fn measure_single_run_proc(
    worker_count: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let shm_name = format!("/saturator_{}_{}", std::process::id(), worker_count);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, worker_count);
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
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(worker_count.to_string())
            .arg(random_access.to_string())
            .arg(direct_io.to_string())
            .spawn()
            .expect("Failed to spawn worker process");
        children.push(child);
    }

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < worker_count as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    // Reset per-worker counters after warmup
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    // Measure
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);

    // Reap children so they flush remaining local_ops
    for mut child in children {
        let _ = child.wait();
    }

    // Sum per-worker counters (global counters are no longer written by workers)
    let mut cpu = 0.0_f64;
    let mut io = 0.0_f64;
    let mut per_worker = Vec::with_capacity(worker_count);
    for i in 0..worker_count {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        cpu += wc;
        io += wi;
        per_worker.push((wc, wi, ws));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, worker_count);

    (cpu, io, metrics, per_worker)
}

/// Collect multiple samples of mixed-intensity process throughput and return aggregated results.
pub fn measure_proc_throughput_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<_> = (0..num_samples).map(|_| {
        measure_single_run_proc_mixed_intensity(
            base_workers, probe_intensity, cpu_iterations, io_iterations,
            duration_secs, io_perc, buffer_kb, io_buffer_kb, sleep_us, warmup_secs, random_access, direct_io,
        )
    }).collect();
    aggregate_samples(samples)
}

/// Run a single measurement with base workers at intensity=1.0 and one probe worker at `probe_intensity`.
pub fn measure_single_run_proc_mixed_intensity(
    base_workers: usize,
    probe_intensity: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    io_perc: f64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let total_workers = base_workers + 1;
    let shm_name = format!("/saturator_{}_{}", std::process::id(), total_workers);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, total_workers);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();

    // Spawn base workers at intensity=1.0
    for i in 0..base_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg("1.0")
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .arg(random_access.to_string())
            .arg(direct_io.to_string())
            .spawn()
            .expect("Failed to spawn base worker process");
        children.push(child);
    }

    // Spawn 1 probe worker at probe_intensity
    let child = Command::new(&exe)
        .arg("__worker")
        .arg(&shm_name)
        .arg(base_workers.to_string())
        .arg(cpu_iterations.to_string())
        .arg(io_iterations.to_string())
        .arg(io_perc.to_string())
        .arg(buffer_kb.to_string())
        .arg(io_buffer_kb.to_string())
        .arg(probe_intensity.to_string())
        .arg(sleep_us.to_string())
        .arg(total_workers.to_string())
        .arg(random_access.to_string())
        .arg(direct_io.to_string())
        .spawn()
        .expect("Failed to spawn probe worker process");
    children.push(child);

    // Wait for all children to finish setup
    while region.ready_count.load(Ordering::Relaxed) < total_workers as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    // Warmup
    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    // Reset per-worker counters after warmup
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);

    for mut child in children {
        let _ = child.wait();
    }

    // Sum per-worker counters (global counters are no longer written by workers)
    let mut cpu = 0.0_f64;
    let mut io = 0.0_f64;
    let mut per_worker = Vec::with_capacity(total_workers);
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        cpu += wc;
        io += wi;
        per_worker.push((wc, wi, ws));
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, total_workers);

    (cpu, io, metrics, per_worker)
}

/// Run a single measurement of baseline+extra workers with different I/O ratios (process mode).
pub fn measure_single_run_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    use std::process::Command;

    let total_workers = baseline_workers + extra_workers;
    let shm_name = format!("/saturator_{}_{}", std::process::id(), total_workers);
    let (region_ptr, shm_fd) = create_shared_region(&shm_name, total_workers);
    let region = unsafe { &*region_ptr };

    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();

    for i in 0..baseline_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg(i.to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(baseline_io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .arg(random_access.to_string())
            .arg(direct_io.to_string())
            .spawn()
            .expect("Failed to spawn baseline worker");
        children.push(child);
    }

    for i in 0..extra_workers {
        let child = Command::new(&exe)
            .arg("__worker")
            .arg(&shm_name)
            .arg((baseline_workers + i).to_string())
            .arg(cpu_iterations.to_string())
            .arg(io_iterations.to_string())
            .arg(extra_io_perc.to_string())
            .arg(buffer_kb.to_string())
            .arg(io_buffer_kb.to_string())
            .arg(intensity.to_string())
            .arg(sleep_us.to_string())
            .arg(total_workers.to_string())
            .arg(random_access.to_string())
            .arg(direct_io.to_string())
            .spawn()
            .expect("Failed to spawn extra worker");
        children.push(child);
    }

    while region.ready_count.load(Ordering::Relaxed) < total_workers as u64 {
        std::thread::sleep(Duration::from_millis(READY_POLL_INTERVAL_MS));
    }

    std::thread::sleep(Duration::from_secs(warmup_secs));
    region.cpu_ops.store(0, Ordering::Relaxed);
    region.io_ops.store(0, Ordering::Relaxed);
    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        wk_cpu.store(0, Ordering::Relaxed);
        wk_io.store(0, Ordering::Relaxed);
        wk_sleep.store(0, Ordering::Relaxed);
    }

    let snap_before = proc_metrics::take_snapshot();
    std::thread::sleep(Duration::from_secs(duration_secs));
    let snap_after = proc_metrics::take_snapshot();
    let metrics = proc_metrics::compute_delta(&snap_before, &snap_after, duration_secs as f64);

    region.running.store(false, Ordering::Relaxed);
    for mut child in children {
        let _ = child.wait();
    }

    let mut baseline_cpu = 0.0_f64;
    let mut baseline_io = 0.0_f64;
    let mut extra_cpu = 0.0_f64;
    let mut extra_io = 0.0_f64;
    let mut per_worker = Vec::with_capacity(total_workers);

    for i in 0..total_workers {
        let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region_ptr, i) };
        let wc = wk_cpu.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let wi = wk_io.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        let ws = wk_sleep.load(Ordering::Relaxed) as f64 / duration_secs as f64;
        per_worker.push((wc, wi, ws));
        if i < baseline_workers {
            baseline_cpu += wc;
            baseline_io += wi;
        } else {
            extra_cpu += wc;
            extra_io += wi;
        }
    }

    destroy_shared_region(&shm_name, region_ptr, shm_fd, total_workers);
    (baseline_cpu, baseline_io, extra_cpu, extra_io, metrics, per_worker)
}

/// Collect multiple samples of process-based slack measurement and return aggregated results.
pub fn measure_proc_slack(
    baseline_workers: usize,
    extra_workers: usize,
    baseline_io_perc: f64,
    extra_io_perc: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    duration_secs: u64,
    buffer_kb: usize,
    io_buffer_kb: usize,
    num_samples: usize,
    intensity: f64,
    sleep_us: u64,
    warmup_secs: u64,
    random_access: bool,
    direct_io: bool,
) -> (f64, f64, f64, f64, f64, f64, proc_metrics::SystemMetrics, Vec<(f64, f64, f64)>) {
    let samples: Vec<_> = (0..num_samples).map(|_| {
        measure_single_run_proc_slack(
            baseline_workers, extra_workers, baseline_io_perc, extra_io_perc,
            cpu_iterations, io_iterations, duration_secs, buffer_kb, io_buffer_kb,
            intensity, sleep_us, warmup_secs, random_access, direct_io,
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
