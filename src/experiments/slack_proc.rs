use crate::saturator::{CalibrationResult, TuningParams};
use crate::proc_metrics;
use crate::measure::{
    measure_proc_slack,
    timestamp, write_params_file,
};

/// Run a process-based slack experiment: measure baseline with N workers, then add extra workers.
pub fn run_slack_proc_experiment(
    baseline_label: &str,
    baseline_io_perc: f64,
    track_io: bool,
    baseline_workers: usize,
    extra_io_perc: f64,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    println!("=== FINDING PROC SLACK ({} BASELINE) ===", baseline_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --max-workers={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb, params.max_workers,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 { print!(", --intensity={:.2}", params.intensity); }
    println!();
    println!("Baseline: {} {}-only processes", baseline_workers, baseline_label);
    println!("Adding processes at: {:.0}% I/O\n", extra_io_perc * 100.0);

    let sleep_us = calibration.cpu_us as u64;

    let (b_cpu, b_io, _, _, b0_cpu_std, b0_io_std, b0_metrics, b0_per_worker) = measure_proc_slack(
        baseline_workers, 0, baseline_io_perc, 0.0,
        calibration.cpu_iterations, calibration.io_iterations,
        params.duration_secs, params.buffer_kb, params.io_buffer_kb,
        params.samples, params.intensity, sleep_us, params.warmup_secs, params.random_access, params.direct_io,
    );
    let baseline_throughput = if track_io { b_io } else { b_cpu };
    println!("Baseline: {:.0} {} ops/sec (cpu: {:.0}, io: {:.0})\n",
             baseline_throughput, baseline_label, b_cpu, b_io);

    let csv_base = format!("proc_slack_{}{}proc_adding_{}pct_io",
                           baseline_workers,
                           baseline_label.to_lowercase().replace("/", ""),
                           (extra_io_perc * 100.0) as u32);
    let run_dir = format!("{}_{}", csv_base, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("{} proc slack", baseline_label), params, &calibration, &[
        ("baseline_workers", format!("{}", baseline_workers)),
        ("baseline_io_perc", format!("{}", baseline_io_perc)),
        ("extra_io_perc", format!("{}", extra_io_perc)),
    ]);

    let filename = format!("{}/{}.csv", run_dir, csv_base);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_workers,total_workers,baseline_workers,extra_io_pct,baseline_cpu_ops,baseline_io_ops,extra_cpu_ops,extra_io_ops,total_ops,baseline_change_pct,baseline_cpu_stddev,baseline_io_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    // Write extra=0 baseline row
    writeln!(file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, baseline_workers, baseline_workers,
             (extra_io_perc * 100.0) as u32,
             b_cpu, b_io, 0.0_f64, 0.0_f64, b_cpu + b_io, 0.0,
             b0_cpu_std, b0_io_std,
             b0_metrics.to_csv_row()).unwrap();

    println!("  {:>5} {:>7} | {:>12} {:>12} | {:>12} {:>12} | {:>8} | {:>6} {:>6}",
             "extra", "total", "base_cpu/s", "base_io/s", "extra_cpu/s", "extra_io/s", "vs base", "cpu%", "io%");
    println!("  {}", "-".repeat(105));

    let mut per_worker_rows: Vec<(usize, Vec<(f64, f64, f64)>)> = vec![(baseline_workers, b0_per_worker)];

    for extra in (1..=params.max_workers).step_by(params.step) {
        let (b_cpu, b_io, e_cpu, e_io, b_cpu_std, b_io_std, metrics, per_worker) = measure_proc_slack(
            baseline_workers, extra, baseline_io_perc, extra_io_perc,
            calibration.cpu_iterations, calibration.io_iterations,
            params.duration_secs, params.buffer_kb, params.io_buffer_kb,
            params.samples, params.intensity, sleep_us, params.warmup_secs, params.random_access, params.direct_io,
        );

        let total_ops = b_cpu + b_io + e_cpu + e_io;
        let tracked = if track_io { b_io } else { b_cpu };
        let baseline_change = (tracked - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>5} {:>7} | {:>12.0} {:>12.0} | {:>12.0} {:>12.0} | {:>+7.1}% | {:>5.1}% {:>5.1}%",
                 extra, baseline_workers + extra, b_cpu, b_io, e_cpu, e_io, baseline_change,
                 metrics.cpu_pct, metrics.io_util_pct);

        writeln!(file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 extra, baseline_workers + extra, baseline_workers,
                 (extra_io_perc * 100.0) as u32,
                 b_cpu, b_io, e_cpu, e_io, total_ops, baseline_change,
                 b_cpu_std, b_io_std,
                 metrics.to_csv_row()).unwrap();

        per_worker_rows.push((baseline_workers + extra, per_worker));
    }

    {
        use std::io::Write as _;
        let pw_path = format!("{}/per_worker_{}.csv", run_dir, csv_base);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "extra_workers,total_workers,baseline_workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec").unwrap();
        for (total_workers, per_worker) in &per_worker_rows {
            let extra = total_workers - baseline_workers;
            for (wid, (wc, wi, ws)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2}",
                         extra, total_workers, baseline_workers, wid, wc, wi, ws, wc + wi).unwrap();
            }
        }
    }

    println!("\nResults written to: {}", filename);
}
