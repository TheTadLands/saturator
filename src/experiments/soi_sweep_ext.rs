use std::io::Write as _;

use crate::saturator::TuningParams;
use crate::soi::{SoiType, CacheSizes, soi_buffer_size};
use crate::proc_metrics;
use crate::measure::{
    timestamp, write_params_file,
};
use crate::measure::ext::{measure_ext_throughput, ExtTimeSeriesSample};

/// Run a single SoI sweep with an external victim workload.
pub fn run_soi_sweep_ext_experiment(
    soi_type: SoiType,
    ext_cmd: &str,
    throughput_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
) {
    println!("=== SoI SWEEP (EXTERNAL): {} ({}) ===", soi_type.name(), soi_type.resource());
    println!("External cmd: {}", ext_cmd);
    println!("Throughput file: {}", throughput_file);
    println!("Sweeping {} SoI workers 0..{} by {}", soi_type.name(), params.max_workers, params.step);

    let run_dir = format!("ext_soi_{}_{}", soi_type.name(), timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("ext_soi_sweep_{}", soi_type.name()), params,
        // No calibration for external workloads — pass a dummy
        &crate::saturator::CalibrationResult { cpu_iterations: 0, io_iterations: 0, cpu_us: 0, io_us: 0 },
        &[
            ("soi_type", soi_type.name().to_string()),
            ("soi_resource", soi_type.resource().to_string()),
            ("ext_cmd", ext_cmd.to_string()),
            ("throughput_file", throughput_file.to_string()),
        ]);

    let csv_path = format!("{}/ext_soi_{}_throughput.csv", run_dir, soi_type.name());
    let mut csv_file = std::fs::File::create(&csv_path).unwrap();
    writeln!(csv_file, "soi_workers,ext_ops_sec,ext_change_pct,soi_ops,ext_ops_stddev,total_ops,total_ops_secs,ops_sec_p10,ops_sec_p50,ops_sec_p90,{}",
             proc_metrics::csv_header()).unwrap();

    // Long-format per-sample CSV: one row per (soi_workers, sample_idx).
    // Preserves individual sample values so plots can show within-N distribution
    // rather than only median/stddev.
    let ps_csv_path = format!("{}/per_sample_ext_soi_{}.csv", run_dir, soi_type.name());
    let mut ps_file = std::fs::File::create(&ps_csv_path).unwrap();
    writeln!(ps_file, "soi_workers,sample_idx,ext_ops_sec,soi_ops").unwrap();

    // Time-series CSV
    let mut ts_file = if params.sample_interval_ms.is_some() {
        let ts_path = format!("{}/timeseries_ext_soi_{}.csv", run_dir, soi_type.name());
        let mut f = std::fs::File::create(&ts_path).unwrap();
        writeln!(f, "soi_workers,elapsed_ms,ext_ops_sec,soi_ops_sec,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    println!("\n  {:>7} | {:>12} | {:>8} | {:>10} | {:>6} {:>6} {:>6} {:>6}",
             "soi", "ext ops/s", "change%", "soi ops/s", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(82));

    // Baseline: external workload alone (0 SoI workers)
    let base = measure_ext_throughput(ext_cmd, throughput_file, 0, soi_type, 0, params);
    // Pooled mean rate (total ops / elapsed) is a more defensible summary than
    // the median of per-run rates when the victim is bimodal (e.g. RocksDB
    // fillrandom phases). Falls back to median if the protocol file was empty.
    let base_ext = if base.total_ops_secs > 0.0 {
        base.total_ops / base.total_ops_secs
    } else {
        base.median_ext_ops
    };

    if base_ext == 0.0 {
        eprintln!("WARNING: external workload reported 0 ops/sec. Check that the throughput file is being written.");
    }

    println!("  {:>7} | {:>12.0} | {:>7.1}% | {:>10} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
             0, base_ext, 0.0, "-",
             base.metrics.cpu_pct, base.metrics.io_util_pct, base.metrics.io_iops_util_pct, base.metrics.mem_usage_pct);

    writeln!(csv_file, "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, base_ext, 0.0, 0.0, base.ext_stddev,
             base.total_ops, base.total_ops_secs,
             base.p10_ops_sec, base.p50_ops_sec, base.p90_ops_sec,
             base.metrics.to_csv_row()).unwrap();

    for (i, (e, s)) in base.per_sample_ext_ops.iter().zip(base.per_sample_soi_ops.iter()).enumerate() {
        writeln!(ps_file, "{},{},{:.2},{:.2}", 0, i, e, s).unwrap();
    }

    write_ext_timeseries(&mut ts_file, 0, &base.timeseries);

    // Sweep SoI workers
    let mut soi_count = params.step;
    while soi_count <= params.max_workers {
        let buf_size = soi_buffer_size(soi_type, cache_sizes, soi_count);

        let r = measure_ext_throughput(ext_cmd, throughput_file, soi_count, soi_type, buf_size, params);

        let r_mean = if r.total_ops_secs > 0.0 {
            r.total_ops / r.total_ops_secs
        } else {
            r.median_ext_ops
        };
        let change_pct = if base_ext > 0.0 {
            (r_mean - base_ext) / base_ext * 100.0
        } else {
            0.0
        };

        println!("  {:>7} | {:>12.0} | {:>7.1}% | {:>10.0} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
                 soi_count, r_mean, change_pct, r.median_soi_ops,
                 r.metrics.cpu_pct, r.metrics.io_util_pct, r.metrics.io_iops_util_pct, r.metrics.mem_usage_pct);

        writeln!(csv_file, "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 soi_count, r_mean, change_pct, r.median_soi_ops, r.ext_stddev,
                 r.total_ops, r.total_ops_secs,
                 r.p10_ops_sec, r.p50_ops_sec, r.p90_ops_sec,
                 r.metrics.to_csv_row()).unwrap();

        for (i, (e, s)) in r.per_sample_ext_ops.iter().zip(r.per_sample_soi_ops.iter()).enumerate() {
            writeln!(ps_file, "{},{},{:.2},{:.2}", soi_count, i, e, s).unwrap();
        }

        write_ext_timeseries(&mut ts_file, soi_count, &r.timeseries);

        soi_count += params.step;
    }

    if ts_file.is_some() {
        println!("Time-series written to: {}/timeseries_ext_soi_{}.csv", run_dir, soi_type.name());
    }
    println!("\nResults written to: {}", csv_path);
}

/// Write time-series samples for saturation experiments (keyed by concurrency).
fn write_saturation_timeseries(
    ts_file: &mut Option<std::fs::File>,
    concurrency: usize,
    ts_data: &Option<Vec<ExtTimeSeriesSample>>,
) {
    if let (Some(f), Some(samples)) = (ts_file.as_mut(), ts_data.as_ref()) {
        for s in samples {
            writeln!(f, "{},{},{:.2},{:.2},{}",
                     concurrency, s.elapsed_ms,
                     s.ext_ops_sec, s.soi_ops_sec,
                     s.metrics.to_csv_row()).unwrap();
        }
    }
}

/// Write time-series samples to the CSV file (no-op if sampling is disabled).
fn write_ext_timeseries(
    ts_file: &mut Option<std::fs::File>,
    soi_workers: usize,
    ts_data: &Option<Vec<ExtTimeSeriesSample>>,
) {
    if let (Some(f), Some(samples)) = (ts_file.as_mut(), ts_data.as_ref()) {
        for s in samples {
            writeln!(f, "{},{},{:.2},{:.2},{}",
                     soi_workers, s.elapsed_ms,
                     s.ext_ops_sec, s.soi_ops_sec,
                     s.metrics.to_csv_row()).unwrap();
        }
    }
}

/// Substitute `{N}` in a command template with the given value.
fn substitute_template(template: &str, n: usize) -> String {
    template.replace("{N}", &n.to_string())
}

/// Find the saturation point of an external workload by sweeping the `{N}` template parameter.
/// Returns the concurrency level that produced peak throughput.
pub fn run_ext_saturation_experiment(
    ext_cmd_template: &str,
    throughput_file: &str,
    params: &TuningParams,
) -> usize {
    println!("=== FINDING EXTERNAL WORKLOAD SATURATION POINT ===");
    println!("Command template: {}", ext_cmd_template);
    println!("Sweeping {{N}} from {} to {} by {}", params.step, params.max_workers, params.step);

    let run_dir = format!("ext_saturation_{}", timestamp());
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, "ext_saturation", params,
        &crate::saturator::CalibrationResult { cpu_iterations: 0, io_iterations: 0, cpu_us: 0, io_us: 0 },
        &[
            ("ext_cmd_template", ext_cmd_template.to_string()),
            ("throughput_file", throughput_file.to_string()),
        ]);

    let csv_path = format!("{}/ext_saturation.csv", run_dir);
    let mut csv_file = std::fs::File::create(&csv_path).unwrap();
    writeln!(csv_file, "concurrency,ext_ops_sec,ext_ops_stddev,throughput_per_unit,total_ops,total_ops_secs,ops_sec_p10,ops_sec_p50,ops_sec_p90,{}",
             proc_metrics::csv_header()).unwrap();

    let ps_csv_path = format!("{}/per_sample_ext_saturation.csv", run_dir);
    let mut ps_file = std::fs::File::create(&ps_csv_path).unwrap();
    writeln!(ps_file, "concurrency,sample_idx,ext_ops_sec").unwrap();

    // Time-series CSV
    let mut ts_file = if params.sample_interval_ms.is_some() {
        let ts_path = format!("{}/timeseries_ext_saturation.csv", run_dir);
        let mut f = std::fs::File::create(&ts_path).unwrap();
        writeln!(f, "concurrency,elapsed_ms,ext_ops_sec,soi_ops_sec,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    println!("\n  {:>7} | {:>12} | {:>12} | {:>6} {:>6} {:>6} {:>6}",
             "N", "ext ops/s", "per unit", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(72));

    let mut best_throughput = 0.0_f64;
    let mut saturation_point = params.step;

    let mut n = params.step;
    while n <= params.max_workers {
        let cmd = substitute_template(ext_cmd_template, n);

        // soi_count=0: no SoI workers, just measure the external workload
        let r = measure_ext_throughput(&cmd, throughput_file, 0, SoiType::Cpu, 0, params);

        let r_mean = if r.total_ops_secs > 0.0 {
            r.total_ops / r.total_ops_secs
        } else {
            r.median_ext_ops
        };
        let per_unit = r_mean / n as f64;

        println!("  {:>7} | {:>12.0} | {:>12.0} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
                 n, r_mean, per_unit,
                 r.metrics.cpu_pct, r.metrics.io_util_pct, r.metrics.io_iops_util_pct, r.metrics.mem_usage_pct);

        writeln!(csv_file, "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 n, r_mean, r.ext_stddev, per_unit,
                 r.total_ops, r.total_ops_secs,
                 r.p10_ops_sec, r.p50_ops_sec, r.p90_ops_sec,
                 r.metrics.to_csv_row()).unwrap();

        for (i, e) in r.per_sample_ext_ops.iter().enumerate() {
            writeln!(ps_file, "{},{},{:.2}", n, i, e).unwrap();
        }

        write_saturation_timeseries(&mut ts_file, n, &r.timeseries);

        if r_mean > best_throughput {
            best_throughput = r_mean;
            saturation_point = n;
        }

        n += params.step;
    }

    if ts_file.is_some() {
        println!("Time-series written to: {}/timeseries_ext_saturation.csv", run_dir);
    }
    println!("\n=== RESULTS ===");
    println!("Saturation point: N={}", saturation_point);
    println!("Best throughput: {:.0} ops/sec", best_throughput);
    println!("Results written to: {}", csv_path);

    saturation_point
}

/// Run external workload saturation finding, then optionally chain into SoI sweep.
pub fn run_ext_saturation_and_sweep(
    soi_types: &[SoiType],
    ext_cmd_template: &str,
    throughput_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
) {
    let saturation_n = run_ext_saturation_experiment(ext_cmd_template, throughput_file, params);

    if params.chain {
        let fixed_cmd = substitute_template(ext_cmd_template, saturation_n);
        println!("\n=== CHAINING: SoI sweep at saturation point N={} ===\n", saturation_n);
        run_soi_ext_experiments(soi_types, &fixed_cmd, throughput_file, cache_sizes, params);
    }
}

/// Run SoI sweeps for multiple SoI types with an external victim workload.
pub fn run_soi_ext_experiments(
    soi_types: &[SoiType],
    ext_cmd: &str,
    throughput_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
) {
    println!("=== SoI INTERFERENCE PROFILING — EXTERNAL WORKLOAD ===");
    println!("External cmd: {}", ext_cmd);
    println!("Cache sizes: L1d={}KB, L2={}KB, L3={}KB",
             cache_sizes.l1d / 1024, cache_sizes.l2 / 1024, cache_sizes.l3 / 1024);
    println!("SoI types: {}\n", soi_types.iter().map(|s| s.name()).collect::<Vec<_>>().join(", "));

    for &soi_type in soi_types {
        run_soi_sweep_ext_experiment(soi_type, ext_cmd, throughput_file, cache_sizes, params);
        println!();
    }

    println!("=== ALL EXTERNAL SoI SWEEPS COMPLETE ===");
}
