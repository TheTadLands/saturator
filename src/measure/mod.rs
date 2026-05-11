pub mod thread;
pub mod proc;
pub mod ext_throughput;
pub mod ext_stats;
pub mod ext;

pub use thread::*;
pub use self::proc::*;

/// A single time-series sample collected during a measurement window.
pub struct TimeSeriesSample {
    pub elapsed_ms: u64,
    pub victim_cpu_ops_sec: f64,
    pub victim_io_ops_sec: f64,
    pub soi_ops_sec: f64,
    pub metrics: crate::proc_metrics::SystemMetrics,
}

/// Compute the median of a slice of f64 values.
pub fn median(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

/// Compute the sample standard deviation of a slice of f64 values.
pub fn stddev(samples: &[f64]) -> f64 {
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let variance = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    variance.sqrt()
}

/// Generate a UTC timestamp string in `YYYYMMDD_HHMMSS` format.
pub fn timestamp() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let s = secs;
    let days = s / 86400;
    let time = s % 86400;
    let hours = time / 3600;
    let minutes = (time % 3600) / 60;
    let seconds = time % 60;
    // Days since epoch to Y/M/D
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for &md in &month_days {
        if remaining < md as i64 { break; }
        remaining -= md as i64;
        m += 1;
    }
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", y, m + 1, remaining + 1, hours, minutes, seconds)
}

/// Write experiment parameters to a `params.txt` file in the given directory.
pub fn write_params_file(
    dir: &str,
    experiment: &str,
    params: &crate::saturator::TuningParams,
    calibration: &crate::saturator::CalibrationResult,
    extra: &[(&str, String)],
) {
    use std::io::Write;
    let path = format!("{}/params.txt", dir);
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, "experiment: {}", experiment).unwrap();
    writeln!(f, "buffer_kb: {}", params.buffer_kb).unwrap();
    writeln!(f, "io_buffer_kb: {}", params.io_buffer_kb).unwrap();
    writeln!(f, "max_workers: {}", params.max_workers).unwrap();
    writeln!(f, "duration_secs: {}", params.duration_secs).unwrap();
    writeln!(f, "samples: {}", params.samples).unwrap();
    writeln!(f, "step: {}", params.step).unwrap();
    writeln!(f, "intensity: {}", params.intensity).unwrap();
    writeln!(f, "warmup_secs: {}", params.warmup_secs).unwrap();
    writeln!(f, "cooldown_secs: {}", params.cooldown_secs).unwrap();
    writeln!(f, "random_access: {}", params.random_access).unwrap();
    writeln!(f, "direct_io: {}", params.direct_io).unwrap();
    if let Some(interval) = params.sample_interval_ms {
        writeln!(f, "sample_interval_ms: {}", interval).unwrap();
    }
    if let Some(nice) = params.nice {
        writeln!(f, "nice: {}", nice).unwrap();
    }
    if let Some(ref prefill) = params.ext_prefill {
        writeln!(f, "prefill: {}", prefill).unwrap();
    }
    writeln!(f, "cpu_iterations: {}", calibration.cpu_iterations).unwrap();
    writeln!(f, "io_iterations: {}", calibration.io_iterations).unwrap();
    writeln!(f, "cpu_us: {}", calibration.cpu_us).unwrap();
    writeln!(f, "io_us: {}", calibration.io_us).unwrap();
    for (key, val) in extra {
        writeln!(f, "{}: {}", key, val).unwrap();
    }
}

/// Remove leftover `/tmp/saturator*` scratch files from previous runs.
pub fn cleanup_scratch_files() {
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

/// Given multiple samples of (cpu_ops, io_ops, metrics, per_worker_data), return
/// (median_cpu, median_io, stddev_cpu, stddev_io, median_metrics, best_per_worker).
///
/// Selects the per-worker data from the sample whose total throughput is closest to the median.
pub fn aggregate_samples(
    samples: Vec<(f64, f64, crate::proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>)>,
) -> (f64, f64, f64, f64, crate::proc_metrics::SystemMetrics, Vec<(f64, f64, f64, u64)>) {
    let cpu_vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let io_vals: Vec<f64> = samples.iter().map(|s| s.1).collect();

    let median_total = median(&cpu_vals) + median(&io_vals);
    let median_idx = samples.iter().enumerate()
        .min_by(|(_, a), (_, b)| {
            let da = ((a.0 + a.1) - median_total).abs();
            let db = ((b.0 + b.1) - median_total).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    let per_worker = samples[median_idx].3.clone();

    let metrics_list: Vec<crate::proc_metrics::SystemMetrics> = samples.into_iter().map(|s| s.2).collect();
    (median(&cpu_vals), median(&io_vals), stddev(&cpu_vals), stddev(&io_vals), crate::proc_metrics::median_metrics(&metrics_list), per_worker)
}
