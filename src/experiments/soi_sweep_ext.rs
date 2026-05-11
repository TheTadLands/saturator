use std::io::Write as _;

use crate::saturator::TuningParams;
use crate::soi::{SoiType, CacheSizes, soi_buffer_size};
use crate::proc_metrics;
use crate::measure::{
    timestamp, write_params_file,
};
use crate::measure::ext::{measure_ext_throughput, run_prefill_blocking, ExtAggregateResult, ExtTimeSeriesSample};

/// Run a single SoI sweep with an external victim workload. The `baseline`
/// argument provides a pre-computed soi_workers=0 measurement so the sweep
/// can skip re-measuring the workload in isolation. The caller is responsible
/// for measuring once and sharing across SoI types.
pub fn run_soi_sweep_ext_experiment(
    soi_type: SoiType,
    ext_cmd: &str,
    throughput_file: &str,
    stats_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
    baseline: &ExtAggregateResult,
) {
    println!("=== SoI SWEEP (EXTERNAL): {} ({}) ===", soi_type.name(), soi_type.resource());
    println!("External cmd: {}", ext_cmd);
    println!("Throughput file: {}", throughput_file);
    println!("Stats file: {}", stats_file);
    println!("Sweeping {} SoI workers 0..{} by {}", soi_type.name(), params.max_workers, params.step);

    if let Some(period_ms) = params.soi_period_ms {
        println!("SoI square-wave: period={}ms, duty={:.0}%", period_ms, params.soi_duty * 100.0);
        if let Some(interval_ms) = params.sample_interval_ms {
            if period_ms % interval_ms != 0 {
                eprintln!("WARNING: sample_interval_ms ({}) does not evenly divide soi_period_ms ({}). \
                           Time-series samples may straddle phase boundaries.", interval_ms, period_ms);
            }
        }
        if matches!(soi_type.backend(), crate::soi::SoiBackend::External(_)) {
            eprintln!("WARNING: --soi-period has no effect on external SoI backend ({})", soi_type.name());
        }
    }

    let run_dir = params.run_dir(&format!("ext_soi_{}_{}", soi_type.name(), timestamp()));
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, &format!("ext_soi_sweep_{}", soi_type.name()), params,
        // No calibration for external workloads — pass a dummy
        &crate::saturator::CalibrationResult { cpu_iterations: 0, io_iterations: 0, cpu_us: 0, io_us: 0 },
        &[
            ("soi_type", soi_type.name().to_string()),
            ("soi_resource", soi_type.resource().to_string()),
            ("ext_cmd", ext_cmd.to_string()),
            ("throughput_file", throughput_file.to_string()),
            ("stats_file", stats_file.to_string()),
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
        writeln!(f, "soi_workers,elapsed_ms,ext_ops_sec,soi_ops_sec,soi_gate_on,victim_gate_on,victim_io_phase,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    // Workload stats CSV (narrow format: one row per metric observation).
    // Always created; stays header-only for workloads that emit no stats.
    let stats_csv_path = format!("{}/ext_stats_{}.csv", run_dir, soi_type.name());
    let mut stats_csv = std::fs::File::create(&stats_csv_path).unwrap();
    writeln!(stats_csv, "soi_workers,sample_idx,timestamp_ms,metric,value").unwrap();

    println!("\n  {:>7} | {:>12} | {:>8} | {:>10} | {:>6} {:>6} {:>6} {:>6}",
             "soi", "ext ops/s", "change%", "soi ops/s", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(82));

    let base_ext = if baseline.total_ops_secs > 0.0 {
        baseline.total_ops / baseline.total_ops_secs
    } else {
        baseline.median_ext_ops
    };

    if base_ext == 0.0 {
        eprintln!("WARNING: external workload reported 0 ops/sec. Check that the throughput file is being written.");
    }

    println!("  {:>7} | {:>12.0} | {:>7.1}% | {:>10} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
             0, base_ext, 0.0, "(cached)",
             baseline.metrics.cpu_pct, baseline.metrics.io_util_pct, baseline.metrics.io_iops_util_pct, baseline.metrics.mem_usage_pct);

    writeln!(csv_file, "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, base_ext, 0.0, 0.0, baseline.ext_stddev,
             baseline.total_ops, baseline.total_ops_secs,
             baseline.p10_ops_sec, baseline.p50_ops_sec, baseline.p90_ops_sec,
             baseline.metrics.to_csv_row()).unwrap();

    for (i, (e, s)) in baseline.per_sample_ext_ops.iter().zip(baseline.per_sample_soi_ops.iter()).enumerate() {
        writeln!(ps_file, "{},{},{:.2},{:.2}", 0, i, e, s).unwrap();
    }

    write_ext_timeseries(&mut ts_file, 0, &baseline.timeseries);
    write_ext_stats(&mut stats_csv, 0, baseline);

    // Sweep SoI workers
    let mut soi_count = params.step;
    while soi_count <= params.max_workers {
        let buf_size = soi_buffer_size(soi_type, cache_sizes, soi_count);

        let r = measure_ext_throughput(ext_cmd, throughput_file, stats_file, soi_count, soi_type, buf_size, params);

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
        write_ext_stats(&mut stats_csv, soi_count, &r);

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
            writeln!(f, "{},{},{:.2},{:.2},{},{},{:.3},{}",
                     concurrency, s.elapsed_ms,
                     s.ext_ops_sec, s.soi_ops_sec,
                     if s.soi_gate_on { 1 } else { 0 },
                     if s.victim_gate_on { 1 } else { 0 },
                     s.victim_io_phase,
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
            writeln!(f, "{},{},{:.2},{:.2},{},{},{:.3},{}",
                     soi_workers, s.elapsed_ms,
                     s.ext_ops_sec, s.soi_ops_sec,
                     if s.soi_gate_on { 1 } else { 0 },
                     if s.victim_gate_on { 1 } else { 0 },
                     s.victim_io_phase,
                     s.metrics.to_csv_row()).unwrap();
        }
    }
}

/// Write per-sample workload stats rows keyed by `soi_workers` (or concurrency,
/// for saturation experiments). Metric names are emitted verbatim; metric
/// values containing embedded commas would break the CSV, so workloads must
/// use simple identifiers.
fn write_ext_stats(
    stats_csv: &mut std::fs::File,
    group_key: usize,
    result: &ExtAggregateResult,
) {
    for (sample_idx, rows) in result.per_sample_stats.iter().enumerate() {
        for row in rows {
            writeln!(stats_csv, "{},{},{},{},{}",
                     group_key, sample_idx, row.timestamp_ms, row.metric, row.value).unwrap();
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
    stats_file: &str,
    params: &TuningParams,
) -> (usize, ExtAggregateResult) {
    println!("=== FINDING EXTERNAL WORKLOAD SATURATION POINT ===");
    println!("Command template: {}", ext_cmd_template);
    println!("Sweeping {{N}} from {} to {} by {}", params.step, params.max_workers, params.step);

    let run_dir = params.run_dir(&format!("ext_saturation_{}", timestamp()));
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, "ext_saturation", params,
        &crate::saturator::CalibrationResult { cpu_iterations: 0, io_iterations: 0, cpu_us: 0, io_us: 0 },
        &[
            ("ext_cmd_template", ext_cmd_template.to_string()),
            ("throughput_file", throughput_file.to_string()),
            ("stats_file", stats_file.to_string()),
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
        writeln!(f, "concurrency,elapsed_ms,ext_ops_sec,soi_ops_sec,soi_gate_on,victim_gate_on,victim_io_phase,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    // Workload stats CSV (same protocol as SoI sweep, keyed by concurrency).
    let stats_csv_path = format!("{}/ext_stats_saturation.csv", run_dir);
    let mut stats_csv = std::fs::File::create(&stats_csv_path).unwrap();
    writeln!(stats_csv, "concurrency,sample_idx,timestamp_ms,metric,value").unwrap();

    println!("\n  {:>7} | {:>12} | {:>12} | {:>6} {:>6} {:>6} {:>6}",
             "N", "ext ops/s", "per unit", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(72));

    let mut best_throughput = 0.0_f64;
    let mut saturation_point = params.step;
    let mut best_result: Option<ExtAggregateResult> = None;

    let mut n = params.step;
    while n <= params.max_workers {
        let cmd = substitute_template(ext_cmd_template, n);

        // soi_count=0: no SoI workers, just measure the external workload
        let r = measure_ext_throughput(&cmd, throughput_file, stats_file, 0, SoiType::Cpu, 0, params);

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
        write_ext_stats(&mut stats_csv, n, &r);

        if r_mean > best_throughput {
            best_throughput = r_mean;
            saturation_point = n;
            best_result = Some(r);
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

    (saturation_point, best_result.expect("no measurements were taken"))
}

/// Run external workload saturation finding, then optionally chain into SoI sweep.
pub fn run_ext_saturation_and_sweep(
    soi_types: &[SoiType],
    ext_cmd_template: &str,
    throughput_file: &str,
    stats_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
) {
    run_prefill_blocking(&substitute_template(ext_cmd_template, 1), throughput_file, stats_file, params);

    let (saturation_n, best_result) = run_ext_saturation_experiment(ext_cmd_template, throughput_file, stats_file, params);

    if params.chain {
        let fixed_cmd = substitute_template(ext_cmd_template, saturation_n);
        println!("\n=== CHAINING: SoI sweep at saturation point N={} (reusing baseline) ===\n", saturation_n);
        run_soi_ext_experiments(soi_types, &fixed_cmd, throughput_file, stats_file, cache_sizes, params, Some(best_result));
    }
}

/// Run SoI sweeps for multiple SoI types with an external victim workload.
/// If `baseline` is provided (e.g. from a preceding saturation sweep), it is
/// reused as the soi_workers=0 reference for every SoI type. Otherwise a single
/// baseline measurement is taken once and shared across all types.
pub fn run_soi_ext_experiments(
    soi_types: &[SoiType],
    ext_cmd: &str,
    throughput_file: &str,
    stats_file: &str,
    cache_sizes: &CacheSizes,
    params: &TuningParams,
    baseline: Option<ExtAggregateResult>,
) {
    println!("=== SoI INTERFERENCE PROFILING — EXTERNAL WORKLOAD ===");
    println!("External cmd: {}", ext_cmd);
    println!("Cache sizes: L1d={}KB, L2={}KB, L3={}KB",
             cache_sizes.l1d / 1024, cache_sizes.l2 / 1024, cache_sizes.l3 / 1024);
    println!("SoI types: {}\n", soi_types.iter().map(|s| s.name()).collect::<Vec<_>>().join(", "));

    run_prefill_blocking(ext_cmd, throughput_file, stats_file, params);

    let baseline = baseline.unwrap_or_else(|| {
        println!("Measuring baseline (0 SoI workers)...");
        measure_ext_throughput(ext_cmd, throughput_file, stats_file, 0, SoiType::Cpu, 0, params)
    });

    for &soi_type in soi_types {
        run_soi_sweep_ext_experiment(soi_type, ext_cmd, throughput_file, stats_file, cache_sizes, params, &baseline);
        println!();
    }

    println!("=== ALL EXTERNAL SoI SWEEPS COMPLETE ===");
}
