use crate::constants::*;
use crate::saturator::{CalibrationResult, TuningParams};
use crate::proc_metrics;
use crate::measure::{
    measure_proc_throughput_mixed_intensity,
    timestamp, write_params_file,
};

/// Run an intensity sweep: N base workers at intensity=1.0, sweep 1 probe worker from 0.0 to 1.0.
pub fn run_intensity_sweep_experiment(
    base_workers: usize,
    io_perc: f64,
    io_pct_int: u32,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    println!("=== INTENSITY SWEEP (PROCESS MODE) ===");
    println!("{} base workers at intensity=1.0, io_pct={}%", base_workers, io_pct_int);
    println!("Sweeping 1 probe worker intensity from 0.0 to 1.0\n");

    let sleep_us = calibration.cpu_us as u64;

    let run_dir = format!("proc_intensity_sweep_{}base_{}pct_io_{}", base_workers, io_pct_int, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, "intensity sweep", params, &calibration, &[
        ("base_workers", format!("{}", base_workers)),
        ("io_pct", format!("{}", io_pct_int)),
        ("io_perc", format!("{}", io_perc)),
    ]);

    let csv_path = format!("{}/proc_intensity_sweep_{}base_{}pct_io.csv", run_dir, base_workers, io_pct_int);
    let mut file = std::fs::File::create(&csv_path).unwrap();
    use std::io::Write as _;
    writeln!(file, "probe_intensity,workers,cpu_ops_sec,io_ops_sec,total_ops_sec,cpu_ops_stddev,io_ops_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    println!("  {:>9} | {:>12} {:>12} {:>12} | {:>6} {:>6}",
             "intensity", "cpu ops/s", "io ops/s", "total ops/s", "cpu%", "io%");
    println!("  {}", "-".repeat(72));

    let mut best_throughput = 0.0;
    let mut best_intensity = 0.0;
    let total_workers = base_workers + 1;
    let mut per_worker_rows: Vec<(f64, Vec<(f64, f64, f64)>)> = Vec::new();

    for step in 0..=INTENSITY_SWEEP_STEPS {
        let probe_intensity = step as f64 * (1.0 / INTENSITY_SWEEP_STEPS as f64);

        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_worker) = measure_proc_throughput_mixed_intensity(
            base_workers, probe_intensity, calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, io_perc, params.buffer_kb, params.io_buffer_kb, params.samples,
            sleep_us, params.warmup_secs,
        );
        let total_ops = cpu_ops + io_ops;

        println!("  {:>9.2} | {:>12.0} {:>12.0} {:>12.0} | {:>5.1}% {:>5.1}%",
                 probe_intensity, cpu_ops, io_ops, total_ops,
                 metrics.cpu_pct, metrics.io_util_pct);

        writeln!(file, "{:.2},{},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 probe_intensity, total_workers, cpu_ops, io_ops, total_ops,
                 cpu_stddev, io_stddev, metrics.to_csv_row()).unwrap();

        per_worker_rows.push((probe_intensity, per_worker));

        if total_ops > best_throughput {
            best_throughput = total_ops;
            best_intensity = probe_intensity;
        }
    }

    // Write per-worker CSV
    {
        let pw_path = format!("{}/per_worker_proc_intensity_sweep_{}base_{}pct_io.csv", run_dir, base_workers, io_pct_int);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "probe_intensity,workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (intensity, per_worker) in &per_worker_rows {
            for (wid, (wc, wi, ws)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{:.2},{},{},{:.2},{:.2},{:.2},{:.2}", intensity, total_workers, wid, wc, wi, ws, wc + wi).unwrap();
            }
        }
    }

    println!("\n=== RESULTS ===");
    println!("Best throughput: {:.0} ops/sec at probe intensity {:.2}", best_throughput, best_intensity);
    println!("Results written to: {}", csv_path);
}
