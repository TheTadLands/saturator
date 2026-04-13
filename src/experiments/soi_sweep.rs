use std::io::Write as _;

use crate::saturator::{CalibrationResult, TuningParams};
use crate::soi::{SoiType, CacheSizes, soi_buffer_size};
use crate::proc_metrics;
use crate::measure::{
    measure_soi_throughput,
    timestamp, write_params_file,
    TimeSeriesSample,
};

/// Run a single SoI sweep: fixed victim workers, incrementally add SoI workers of one type.
pub fn run_soi_sweep_experiment(
    soi_type: SoiType,
    victim_workers: usize,
    victim_io_perc: f64,
    cache_sizes: &CacheSizes,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    let victim_io_pct_int = (victim_io_perc * 100.0) as u32;

    println!("=== SoI SWEEP: {} ({}) ===", soi_type.name(), soi_type.resource());
    println!("{} victim workers ({}% IO), sweeping {} SoI workers 0..{} by {}",
             victim_workers, victim_io_pct_int, soi_type.name(),
             params.max_workers, params.step);

    let run_dir = format!("soi_{}_{}_victims_{}pct_io_{}",
                          soi_type.name(), victim_workers, victim_io_pct_int, timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("soi_sweep_{}", soi_type.name()), params, &calibration, &[
        ("soi_type", soi_type.name().to_string()),
        ("soi_resource", soi_type.resource().to_string()),
        ("victim_workers", format!("{}", victim_workers)),
        ("victim_io_pct", format!("{}", victim_io_pct_int)),
    ]);

    let csv_path = format!("{}/soi_{}_throughput.csv", run_dir, soi_type.name());
    let mut csv_file = std::fs::File::create(&csv_path).unwrap();
    writeln!(csv_file, "soi_workers,total_workers,victim_workers,soi_type,victim_cpu_ops,victim_io_ops,victim_total_ops,victim_cpu_change_pct,victim_io_change_pct,victim_change_pct,soi_ops,victim_cpu_stddev,victim_io_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    let pw_csv_path = format!("{}/per_worker_soi_{}.csv", run_dir, soi_type.name());
    let mut pw_file = std::fs::File::create(&pw_csv_path).unwrap();
    writeln!(pw_file, "soi_workers,total_workers,worker_id,is_soi,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec,io_errors").unwrap();

    println!("\n  {:>7} {:>7} | {:>12} {:>12} {:>12} | {:>8} | {:>10} | {:>6} {:>6} {:>6} {:>6}",
             "soi", "total", "victim cpu", "victim io", "victim tot", "change%", "soi ops/s", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(114));

    // Time-series CSV (only created when --sample-interval is set)
    let mut ts_file = if params.sample_interval_ms.is_some() {
        let ts_path = format!("{}/timeseries_soi_{}.csv", run_dir, soi_type.name());
        let mut f = std::fs::File::create(&ts_path).unwrap();
        writeln!(f, "soi_workers,elapsed_ms,victim_cpu_ops_sec,victim_io_ops_sec,soi_ops_sec,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    // Measure baseline (0 SoI workers)
    let (base_cpu, base_io, base_cpu_sd, base_io_sd, _soi_ops, base_metrics, base_pw, base_ts) =
        measure_soi_throughput(
            victim_workers, victim_io_perc, 0, soi_type, 0,
            &calibration, params,
        );
    let baseline_total = base_cpu + base_io;

    write_timeseries(&mut ts_file, 0, &base_ts);

    println!("  {:>7} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>7.1}% | {:>10} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
             0, victim_workers, base_cpu, base_io, baseline_total, 0.0, "-",
             base_metrics.cpu_pct, base_metrics.io_util_pct, base_metrics.io_iops_util_pct, base_metrics.mem_usage_pct);

    writeln!(csv_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, victim_workers, victim_workers, soi_type.name(),
             base_cpu, base_io, baseline_total, 0.0, 0.0, 0.0, 0.0,
             base_cpu_sd, base_io_sd, base_metrics.to_csv_row()).unwrap();

    for (wid, &(wc, wi, ws, we)) in base_pw.iter().enumerate() {
        let is_soi = wid >= victim_workers;
        writeln!(pw_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                 0, victim_workers, wid, is_soi, wc, wi, ws, wc + wi, we).unwrap();
    }

    // Sweep SoI workers
    let mut soi_count = params.step;
    while soi_count <= params.max_workers {
        let buf_size = soi_buffer_size(soi_type, cache_sizes, soi_count);
        let total_workers = victim_workers + soi_count;

        let (v_cpu, v_io, v_cpu_sd, v_io_sd, soi_ops, metrics, per_worker, ts_data) =
            measure_soi_throughput(
                victim_workers, victim_io_perc, soi_count, soi_type, buf_size,
                &calibration, params,
            );
        let victim_total = v_cpu + v_io;
        let cpu_change_pct = if base_cpu > 0.0 {
            (v_cpu - base_cpu) / base_cpu * 100.0
        } else {
            0.0
        };
        let io_change_pct = if base_io > 0.0 {
            (v_io - base_io) / base_io * 100.0
        } else {
            0.0
        };
        let change_pct = if baseline_total > 0.0 {
            (victim_total - baseline_total) / baseline_total * 100.0
        } else {
            0.0
        };

        println!("  {:>7} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>7.1}% | {:>10.0} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
                 soi_count, total_workers, v_cpu, v_io, victim_total, change_pct, soi_ops,
                 metrics.cpu_pct, metrics.io_util_pct, metrics.io_iops_util_pct, metrics.mem_usage_pct);

        writeln!(csv_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 soi_count, total_workers, victim_workers, soi_type.name(),
                 v_cpu, v_io, victim_total, cpu_change_pct, io_change_pct, change_pct, soi_ops,
                 v_cpu_sd, v_io_sd, metrics.to_csv_row()).unwrap();

        for (wid, &(wc, wi, ws, we)) in per_worker.iter().enumerate() {
            let is_soi = wid >= victim_workers;
            writeln!(pw_file, "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                     soi_count, total_workers, wid, is_soi, wc, wi, ws, wc + wi, we).unwrap();
        }

        write_timeseries(&mut ts_file, soi_count, &ts_data);

        soi_count += params.step;
    }

    if ts_file.is_some() {
        println!("Time-series written to: {}/timeseries_soi_{}.csv", run_dir, soi_type.name());
    }
    println!("\nResults written to: {}", csv_path);
}

/// Write time-series samples to the CSV file (no-op if sampling is disabled).
fn write_timeseries(
    ts_file: &mut Option<std::fs::File>,
    soi_workers: usize,
    ts_data: &Option<Vec<TimeSeriesSample>>,
) {
    if let (Some(f), Some(samples)) = (ts_file.as_mut(), ts_data.as_ref()) {
        for s in samples {
            writeln!(f, "{},{},{:.2},{:.2},{:.2},{}",
                     soi_workers, s.elapsed_ms,
                     s.victim_cpu_ops_sec, s.victim_io_ops_sec, s.soi_ops_sec,
                     s.metrics.to_csv_row()).unwrap();
        }
    }
}

/// Run SoI sweep experiments for multiple SoI types.
pub fn run_soi_experiments(
    soi_types: &[SoiType],
    victim_workers: usize,
    victim_io_perc: f64,
    cache_sizes: &CacheSizes,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    println!("=== SoI INTERFERENCE PROFILING (PROCESS MODE) ===");
    println!("{} victim workers ({}% IO)", victim_workers, (victim_io_perc * 100.0) as u32);
    println!("Cache sizes: L1d={}KB, L2={}KB, L3={}KB",
             cache_sizes.l1d / 1024, cache_sizes.l2 / 1024, cache_sizes.l3 / 1024);
    println!("SoI types: {}\n", soi_types.iter().map(|s| s.name()).collect::<Vec<_>>().join(", "));

    for &soi_type in soi_types {
        run_soi_sweep_experiment(soi_type, victim_workers, victim_io_perc, cache_sizes, calibration, params);
        println!();
    }

    println!("=== ALL SoI SWEEPS COMPLETE ===");
}
