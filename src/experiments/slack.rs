use crate::saturator::{CalibrationResult, TuningParams};
use crate::proc_metrics;
use crate::measure::{
    measure_thread_throughput, measure_total_throughput,
    timestamp, write_params_file,
};
use super::SlackExperiment;

/// Run a thread-based slack experiment: measure baseline throughput, then add extra threads.
pub fn run_slack_experiment(
    exp: SlackExperiment,
    calibration: CalibrationResult,
    params: &TuningParams,
    baseline_threads: usize,
    extra_io_ratio: f64,
) {
    println!("=== FINDING SLACK ({} BASELINE) ===", exp.baseline_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 { print!(", --intensity={:.2}", params.intensity); }
    println!();
    println!("Baseline: {} {}-only threads", baseline_threads, exp.baseline_label);
    println!("Adding threads at: {:.0}% I/O\n", extra_io_ratio * 100.0);

    println!("Measuring baseline {} throughput...", exp.baseline_label);
    let (baseline_cpu, baseline_io, baseline_cpu_std, baseline_io_std, baseline_metrics, baseline_per_thread) = measure_thread_throughput(
        baseline_threads, exp.baseline_io_ratio, &calibration, params,
    );
    let baseline_throughput = if exp.track_io { baseline_io } else { baseline_cpu };
    println!("Baseline: {:.0} {} ops/sec (cpu: {:.0}, io: {:.0})\n",
             baseline_throughput, exp.baseline_label, baseline_cpu, baseline_io);

    let logical_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_extra = logical_cores.max(baseline_threads * 2);
    let mut best_tracked_ops = baseline_throughput;
    let mut best_extra_count = 0;

    let csv_base = format!("slack_{}{}_adding_{}pct_io",
                           baseline_threads,
                           exp.baseline_label.to_lowercase(),
                           (extra_io_ratio * 100.0) as u32);
    let run_dir = format!("{}_{}", csv_base, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("{} slack", exp.baseline_label), params, &calibration, &[
        ("baseline_threads", format!("{}", baseline_threads)),
        ("baseline_io_ratio", format!("{}", exp.baseline_io_ratio)),
        ("extra_io_ratio", format!("{}", extra_io_ratio)),
    ]);

    let filename = format!("{}/{}.csv", run_dir, csv_base);
    let mut file = std::fs::File::create(&filename).unwrap();
    use std::io::Write;
    writeln!(file, "extra_threads,total_threads,extra_io_pct,cpu_ops,io_ops,total_ops,baseline_change_pct,cpu_ops_stddev,io_ops_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    // Write extra=0 baseline row
    writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, baseline_threads,
             (extra_io_ratio * 100.0) as u32,
             baseline_cpu, baseline_io, baseline_cpu + baseline_io, 0.0,
             baseline_cpu_std, baseline_io_std,
             baseline_metrics.to_csv_row()).unwrap();

    println!("  {:>5} {:>7} | {:>12} {:>12} {:>12} | {:>8} | {:>6} {:>6} {:>6}",
             "extra", "total", "cpu ops/s", "io ops/s", "total ops/s", "vs base", "cpu%", "io%", "iops%");
    println!("  {}", "-".repeat(97));

    // Per-thread CSV: seed with extra=0 baseline data
    let pw_filename = format!("{}/per_worker_{}.csv", run_dir, csv_base);
    let mut pw_file = {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&pw_filename).unwrap();
        writeln!(f, "extra_threads,total_threads,thread_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec,io_errors").unwrap();
        for (tid, (wc, wi, ws, we)) in baseline_per_thread.iter().enumerate() {
            writeln!(f, "{},{},{},{:.2},{:.2},{:.2},{:.2},{}", 0, baseline_threads, tid, wc, wi, ws, wc + wi, we).unwrap();
        }
        f
    };

    for extra in 1..=max_extra {
        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_thread) = measure_total_throughput(
            baseline_threads, extra, exp.baseline_io_ratio, extra_io_ratio,
            &calibration, params,
        );

        let total_ops = cpu_ops + io_ops;
        let tracked_ops = if exp.track_io { io_ops } else { cpu_ops };
        let baseline_change = (tracked_ops - baseline_throughput) / baseline_throughput * 100.0;

        println!("  {:>5} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>+7.1}% | {:>5.1}% {:>5.1}% {:>5.1}%",
                 extra, baseline_threads + extra, cpu_ops, io_ops, total_ops, baseline_change,
                 metrics.cpu_pct, metrics.io_util_pct, metrics.io_iops_util_pct);

        writeln!(file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 extra, baseline_threads + extra,
                 (extra_io_ratio * 100.0) as u32,
                 cpu_ops, io_ops, total_ops, baseline_change,
                 cpu_stddev, io_stddev,
                 metrics.to_csv_row()).unwrap();

        use std::io::Write as _;
        for (tid, (wc, wi, ws, we)) in per_thread.iter().enumerate() {
            writeln!(pw_file, "{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                     extra, baseline_threads + extra, tid, wc, wi, ws, wc + wi, we).unwrap();
        }

        if tracked_ops > best_tracked_ops {
            best_tracked_ops = tracked_ops;
            best_extra_count = extra;
        }
    }

    println!("\n=== RESULTS ===");
    println!("Baseline: {} threads = {:.0} {} ops/sec",
             baseline_threads, baseline_throughput, exp.baseline_label);
    println!("Best {}: {} threads = {:.0} {} ops/sec ({:+.1}%)",
             exp.tracked_label,
             baseline_threads + best_extra_count, best_tracked_ops, exp.tracked_label,
             (best_tracked_ops - baseline_throughput) / baseline_throughput * 100.0);
    println!("Results written to: {}", filename);
}
