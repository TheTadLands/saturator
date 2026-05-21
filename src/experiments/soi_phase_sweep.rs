use std::io::Write as _;

use crate::saturator::{CalibrationResult, TuningParams};
use crate::soi::{SoiType, CacheSizes, SoiBackend, soi_buffer_size};
use crate::proc_metrics;
use crate::measure::{
    measure_soi_phased_throughput,
    timestamp, write_params_file,
    TimeSeriesSample,
};

/// Run a phased SoI sweep: victim workers cycle through IO% phases while multiple
/// SoI types run simultaneously, each gated to activate during a specific victim phase.
pub fn run_soi_phase_sweep_experiment(
    phase_map: &[(SoiType, f64)],
    victim_workers: usize,
    victim_io_perc: f64,
    cache_sizes: &CacheSizes,
    calibration: CalibrationResult,
    params: &TuningParams,
) {
    let victim_io_pct_int = (victim_io_perc * 100.0) as u32;
    let map_label: String = phase_map.iter()
        .map(|(soi, phase)| format!("{}:{:.0}", soi.name(), phase * 100.0))
        .collect::<Vec<_>>()
        .join("+");

    println!("=== SoI PHASE-MATCHED SWEEP ===");
    println!("{} victim workers ({}% IO base), phase map: {}", victim_workers, victim_io_pct_int, map_label);
    println!("Overhead equalization: {} total SoI slots per step (active + idle)", params.max_workers * phase_map.len());

    if let Some(period_ms) = params.victim_period_ms {
        if let Some(ref phases) = params.victim_phases {
            let labels: Vec<String> = phases.iter().map(|p| format!("{:.0}%", p * 100.0)).collect();
            println!("Victim phase cycling: period={}ms, phases=[{}]", period_ms, labels.join(", "));
        }
    }

    for &(soi_type, _) in phase_map {
        if matches!(soi_type.backend(), SoiBackend::External(_)) {
            eprintln!("WARNING: {} uses external backend (fio) — phase-gating will use internal IO worker instead", soi_type.name());
        }
    }

    let map_name = phase_map.iter()
        .map(|(soi, phase)| format!("{}{:.0}", soi.name(), phase * 100.0))
        .collect::<Vec<_>>()
        .join("_");
    let ts = timestamp();
    let run_dir_base = format!("soi_phase_{}_{}v_{}pct_{}", map_name, victim_workers, victim_io_pct_int, ts);
    let run_dir = params.run_dir(&run_dir_base);
    std::fs::create_dir_all(&run_dir).unwrap();

    write_params_file(&run_dir, "soi_phase_sweep", params, &calibration, &[
        ("phase_map", map_label.clone()),
        ("victim_workers", victim_workers.to_string()),
        ("victim_io_pct", victim_io_pct_int.to_string()),
    ]);

    let csv_path = format!("{}/soi_phase_throughput.csv", run_dir);
    let mut csv_file = std::fs::File::create(&csv_path).unwrap();
    writeln!(csv_file, "soi_per_type,total_soi,total_workers,victim_workers,phase_map,victim_cpu_ops,victim_io_ops,victim_total_ops,victim_cpu_change_pct,victim_io_change_pct,victim_change_pct,soi_ops,soi_cpu_ops,soi_io_ops,victim_cpu_stddev,victim_io_stddev,{}",
             proc_metrics::csv_header()).unwrap();

    let pw_csv_path = format!("{}/per_worker_soi_phase.csv", run_dir);
    let mut pw_file = std::fs::File::create(&pw_csv_path).unwrap();
    writeln!(pw_file, "soi_per_type,total_workers,worker_id,role,soi_type,active_phase,cpu_ops_sec,io_ops_sec,sleep_ops_sec,total_ops_sec,io_errors").unwrap();

    println!("\n  {:>7} {:>7} {:>7} | {:>12} {:>12} {:>12} | {:>8} | {:>10} | {:>6} {:>6} {:>6} {:>6}",
             "per_ty", "soi", "total", "victim cpu", "victim io", "victim tot", "change%", "soi ops/s", "cpu%", "io%", "iops%", "mem%");
    println!("  {}", "-".repeat(125));

    let mut ts_file = if params.sample_interval_ms.is_some() {
        let ts_path = format!("{}/timeseries_soi_phase.csv", run_dir);
        let mut f = std::fs::File::create(&ts_path).unwrap();
        writeln!(f, "soi_per_type,sample_idx,elapsed_ms,victim_cpu_ops_sec,victim_io_ops_sec,soi_ops_sec,soi_cpu_ops_sec,soi_io_ops_sec,soi_gate_on,victim_gate_on,victim_io_phase,{}",
                 proc_metrics::csv_header()).unwrap();
        Some(f)
    } else {
        None
    };

    // Baseline: 0 SoI workers (all slots are idle for overhead equalization)
    let n_groups = phase_map.len();
    let max_soi_total = params.max_workers * n_groups;
    let empty_groups: Vec<(SoiType, usize, usize, Option<u64>)> = Vec::new();
    let (base_cpu, base_io, base_cpu_sd, base_io_sd, _soi_ops, _soi_cpu, _soi_io, base_metrics, base_pw, base_ts) =
        measure_soi_phased_throughput(
            victim_workers, victim_io_perc, &empty_groups, max_soi_total, &calibration, params,
        );
    let baseline_total = base_cpu + base_io;

    write_timeseries(&mut ts_file, 0, &base_ts);

    println!("  {:>7} {:>7} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>7.1}% | {:>10} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
             0, 0, victim_workers, base_cpu, base_io, baseline_total, 0.0, "-",
             base_metrics.cpu_pct, base_metrics.io_util_pct, base_metrics.io_iops_util_pct, base_metrics.mem_usage_pct);

    writeln!(csv_file, "{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
             0, 0, victim_workers, victim_workers, map_label,
             base_cpu, base_io, baseline_total, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
             base_cpu_sd, base_io_sd, base_metrics.to_csv_row()).unwrap();

    for (wid, &(wc, wi, ws, we)) in base_pw.iter().enumerate() {
        writeln!(pw_file, "{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                 0, victim_workers, wid, "victim", "", "", wc, wi, ws, wc + wi, we).unwrap();
    }

    // Sweep: add `step` workers of each SoI type per step
    let mut soi_per_type = params.step;
    while soi_per_type <= params.max_workers {
        let total_soi = soi_per_type * n_groups;
        let total_workers = victim_workers + total_soi;

        let soi_groups: Vec<(SoiType, usize, usize, Option<u64>)> = phase_map.iter()
            .map(|&(soi_type, active_phase)| {
                let buf_size = soi_buffer_size(soi_type, cache_sizes, soi_per_type);
                let encoded = (active_phase * 1000.0) as u64;
                (soi_type, buf_size, soi_per_type, Some(encoded))
            })
            .collect();

        let (v_cpu, v_io, v_cpu_sd, v_io_sd, soi_ops, soi_cpu_ops, soi_io_ops, metrics, per_worker, ts_data) =
            measure_soi_phased_throughput(
                victim_workers, victim_io_perc, &soi_groups, max_soi_total, &calibration, params,
            );
        let victim_total = v_cpu + v_io;
        let cpu_change_pct = if base_cpu > 0.0 { (v_cpu - base_cpu) / base_cpu * 100.0 } else { 0.0 };
        let io_change_pct = if base_io > 0.0 { (v_io - base_io) / base_io * 100.0 } else { 0.0 };
        let change_pct = if baseline_total > 0.0 { (victim_total - baseline_total) / baseline_total * 100.0 } else { 0.0 };

        println!("  {:>7} {:>7} {:>7} | {:>12.0} {:>12.0} {:>12.0} | {:>7.1}% | {:>10.0} | {:>5.1}% {:>5.1}% {:>5.1}% {:>5.1}%",
                 soi_per_type, total_soi, total_workers, v_cpu, v_io, victim_total, change_pct, soi_ops,
                 metrics.cpu_pct, metrics.io_util_pct, metrics.io_iops_util_pct, metrics.mem_usage_pct);

        writeln!(csv_file, "{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                 soi_per_type, total_soi, total_workers, victim_workers, map_label,
                 v_cpu, v_io, victim_total, cpu_change_pct, io_change_pct, change_pct, soi_ops, soi_cpu_ops, soi_io_ops,
                 v_cpu_sd, v_io_sd, metrics.to_csv_row()).unwrap();

        for (i, &(wc, wi, ws, we)) in per_worker.iter().enumerate() {
            if i < victim_workers {
                writeln!(pw_file, "{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                         soi_per_type, total_workers, i, "victim", "", "", wc, wi, ws, wc + wi, we).unwrap();
            } else {
                let soi_offset = i - victim_workers;
                let group_idx = soi_offset / soi_per_type;
                let (soi_type, active_phase) = if group_idx < phase_map.len() {
                    (phase_map[group_idx].0.name(), format!("{:.0}", phase_map[group_idx].1 * 100.0))
                } else {
                    ("unknown", String::new())
                };
                writeln!(pw_file, "{},{},{},{},{},{},{:.2},{:.2},{:.2},{:.2},{}",
                         soi_per_type, total_workers, i, "soi", soi_type, active_phase, wc, wi, ws, wc + wi, we).unwrap();
            }
        }

        write_timeseries(&mut ts_file, soi_per_type, &ts_data);

        soi_per_type += params.step;
    }

    if ts_file.is_some() {
        println!("Time-series written to: {}/timeseries_soi_phase.csv", run_dir);
    }
    println!("\nResults written to: {}", csv_path);
}

fn write_timeseries(
    ts_file: &mut Option<std::fs::File>,
    soi_per_type: usize,
    all_ts: &[Option<Vec<TimeSeriesSample>>],
) {
    if let Some(f) = ts_file.as_mut() {
        for (sample_idx, ts_data) in all_ts.iter().enumerate() {
            if let Some(samples) = ts_data {
                for s in samples {
                    writeln!(f, "{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2},{},{},{:.3},{}",
                             soi_per_type, sample_idx, s.elapsed_ms,
                             s.victim_cpu_ops_sec, s.victim_io_ops_sec, s.soi_ops_sec,
                             s.soi_cpu_ops_sec, s.soi_io_ops_sec,
                             if s.soi_gate_on { 1 } else { 0 },
                             if s.victim_gate_on { 1 } else { 0 },
                             s.victim_io_phase,
                             s.metrics.to_csv_row()).unwrap();
                }
            }
        }
    }
}
