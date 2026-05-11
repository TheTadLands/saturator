use crate::saturator::{CalibrationResult, TuningParams};
use crate::visualize::ResultsWriter;
use crate::measure::{
    measure_thread_throughput, measure_proc_throughput,
    timestamp, write_params_file,
};
use super::{SaturationExperiment, Mode};

/// Run a saturation experiment: add workers until throughput plateaus (threads) or degrades (procs).
///
/// Returns `Some((saturation_point, io_perc))` for process-mode experiments (used by `--chain`),
/// or `None` for thread-mode experiments.
pub fn run_saturation_experiment(
    exp: SaturationExperiment,
    calibration: CalibrationResult,
    params: &TuningParams,
) -> Option<(usize, f64)> {
    let (mode_label, worker_label) = match exp.mode {
        Mode::Threads => ("", "threads"),
        Mode::Procs => (" (PROCESS MODE)", "procs"),
    };

    println!("=== FINDING {} SATURATION POINT{} ===", exp.label, mode_label);
    print!("--buffer-kb={}, --io-buffer-kb={}, --max-workers={}, --duration={}, --samples={}, --warmup={}, --step={}",
             params.buffer_kb, params.io_buffer_kb, params.max_workers,
             params.duration_secs, params.samples, params.warmup_secs, params.step);
    if params.intensity < 1.0 {
        print!(", --intensity={:.2}", params.intensity);
    }
    if params.chain {
        print!(", --chain");
    }
    println!();
    println!("Adding {} {} until throughput {}\n",
             exp.label, worker_label,
             if matches!(exp.mode, Mode::Threads) { "plateaus" } else { "degrades" });

    let mut writer = ResultsWriter::new();
    let mut best_throughput = 0.0;
    let mut saturation_point = params.step;
    let mut per_worker_rows: Vec<(usize, Vec<(f64, f64, f64, u64)>)> = Vec::new();

    let use_step = matches!(exp.mode, Mode::Procs);
    let mut worker_count = if use_step { params.step } else { 1 };
    let end = params.max_workers;

    let wlabel = if use_step { "procs" } else { "threads" };
    println!("  {:>7} | {:>12} {:>12} {:>12} | {:>10} | {:>6} {:>6} {:>6} {:>6}",
             wlabel, "cpu ops/s", "io ops/s", "total ops/s", "per worker", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(96));

    while worker_count <= end {
        let (cpu_ops, io_ops, cpu_stddev, io_stddev, metrics, per_worker_data) = match exp.mode {
            Mode::Threads => {
                let (c, i, cs, is, m, pw) = measure_thread_throughput(
                    worker_count, exp.io_perc, &calibration, params,
                );
                (c, i, cs, is, m, Some(pw))
            },
            Mode::Procs => {
                let (c, i, cs, is, m, pw) = measure_proc_throughput(
                    worker_count, exp.io_perc, &calibration, params,
                );
                (c, i, cs, is, m, Some(pw))
            },
        };
        let total_ops = cpu_ops + io_ops;
        let throughput_per_worker = total_ops / worker_count as f64;

        println!("  {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>10.0} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
                 worker_count, cpu_ops, io_ops, total_ops, throughput_per_worker,
                 metrics.cpu_pct, metrics.io_util_pct, metrics.io_iops_util_pct, metrics.mem_usage_pct);

        let io_errors: u64 = per_worker_data.as_ref().map_or(0, |pw| pw.iter().map(|w| w.3).sum());
        writer.add_saturation_point(worker_count, cpu_ops, io_ops, cpu_stddev, io_stddev, io_errors, metrics);

        if let Some(pw) = per_worker_data {
            per_worker_rows.push((worker_count, pw));
        }

        if total_ops > best_throughput {
            best_throughput = total_ops;
            saturation_point = worker_count;
        }

        if use_step {
            worker_count += params.step;
        } else {
            worker_count += 1;
        }
    }

    let run_dir = params.run_dir(&format!("{}_{}", exp.csv_base, timestamp()));
    std::fs::create_dir_all(&run_dir).unwrap();

    let mode_str = match exp.mode { Mode::Threads => "threads", Mode::Procs => "procs" };
    write_params_file(&run_dir, &format!("{} saturation ({})", exp.label, mode_str), params, &calibration, &[
        ("io_perc", format!("{}", exp.io_perc)),
        ("mode", mode_str.to_string()),
    ]);

    let csv_path = format!("{}/{}.csv", run_dir, exp.csv_base);
    writer.write_saturation_csv(std::path::Path::new(&csv_path)).unwrap();

    // Write per-worker CSV (both thread and proc mode)
    if !per_worker_rows.is_empty() {
        use std::io::Write as _;
        let pw_path = format!("{}/per_worker_{}.csv", run_dir, exp.csv_base);
        let mut pw_file = std::fs::File::create(&pw_path).unwrap();
        writeln!(pw_file, "workers,worker_id,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec,io_errors").unwrap();
        for (workers, per_worker) in &per_worker_rows {
            for (wid, (wc, wi, ws, we)) in per_worker.iter().enumerate() {
                writeln!(pw_file, "{},{},{:.2},{:.2},{:.2},{:.2},{}", workers, wid, wc, wi, ws, wc + wi, we).unwrap();
            }
        }
    }

    println!("\n=== RESULTS ===");
    println!("Saturation point: {} {}", saturation_point, worker_label);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    if let Some(rec) = exp.recommendation {
        println!("\nRecommendation: Run `saturator {} {}` to find {} slack",
                 rec, saturation_point,
                 if exp.io_perc < 0.5 { "I/O" } else { "CPU" });
    }

    match exp.mode {
        Mode::Procs => Some((saturation_point, exp.io_perc)),
        Mode::Threads => None,
    }
}
