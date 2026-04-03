mod constants;
mod saturator;
mod visualize;
mod proc_metrics;
mod experiments;
mod measure;
mod soi;

use saturator::{
    calibrate_operations_full, TuningParams,
    run_worker_process,
};
use experiments::{
    SaturationExperiment, SlackExperiment, Mode,
    run_saturation_experiment, run_slack_experiment,
    run_intensity_sweep_experiment, run_slack_proc_experiment,
    run_soi_experiments,
};
use measure::cleanup_scratch_files;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        println!("Usage: saturator <experiment> [args] [OPTIONS]");
        println!("Experiments:");
        println!("  find-saturation                - Find CPU saturation point (threads)");
        println!("  find-io-saturation             - Find I/O saturation point (threads)");
        println!("  find-slack <N> <extra_io%>     - N CPU baseline threads, add threads at extra_io%");
        println!("  find-io-slack <N> <extra_io%>  - N I/O baseline threads, add threads at extra_io%");
        println!("  find-saturation-proc           - Find CPU saturation point (processes)");
        println!("  find-io-saturation-proc        - Find I/O saturation point (processes)");
        println!("  find-mixed-saturation-proc <io_pct> - Mixed CPU+IO processes at given IO% (0-100)");
        println!("  find-saturation-intensity-proc <N> <io_pct> - N base procs + 1 probe, sweep probe intensity");
        println!("  find-slack-proc <N> <extra_io%>    - N CPU baseline procs, add procs at extra_io%");
        println!("  find-io-slack-proc <N> <extra_io%> - N I/O baseline procs, add procs at extra_io%");
        println!("  find-soi-slack <soi|all> <victim_workers> [victim_io%] - SoI interference profiling");
        println!("");
        println!("Options (for -proc variants):");
        println!("  --buffer-kb <N>      CPU work buffer size in KB (default: 100)");
        println!("  --io-buffer-kb <N>   IO read/write buffer size in KB (default: 4)");
        println!("  --max-workers <N>    Max worker count (default: parallelism * 16)");
        println!("  --duration <N>       Measurement duration in seconds (default: 30)");
        println!("  --samples <N>        Samples per data point, median taken (default: 5)");
        println!("  --step <N>           Worker count increment per data point (default: 1)");
        println!("  --intensity <F>      Work probability per iteration, 0.0-1.0 (default: 1.0)");
        println!("  --chain              Auto-run intensity sweep at saturation point (proc only)");
        println!("  --warmup <N>         Warmup duration in seconds before measurement (default: 10)");
        println!("  --random-access      Use random buffer access pattern to defeat hardware prefetcher");
        println!("  --direct-io          Use O_DIRECT to bypass page cache for I/O ops");
        println!("  --sample-interval <ms> Time-series sampling interval in ms (default: off)");
        println!("");
        println!("Examples:");
        println!("  find-slack 4 100     - 4 CPU baseline, add 100% I/O threads");
        println!("  find-saturation-proc --buffer-kb 1024 --max-workers 100 --samples 7");
        println!("  find-mixed-saturation-proc 50 --max-workers 32");
        println!("  find-saturation-intensity-proc 6 50 --duration 2");
        println!("  find-soi-slack l3 4 0 --max-workers 8");
        println!("  find-soi-slack all 4 50 --duration 5 --samples 3");
        return;
    }

    let experiment = &args[1];

    // Hidden worker subcommand — child process entry point
    if experiment == "__worker" {
        // Args: __worker <shm_name> <worker_id> <cpu_iters> <io_iters> <io_perc> <buffer_kb> <io_buffer_kb> <intensity> <sleep_us> <max_workers> <random_access> <direct_io>
        let shm_name = &args[2];
        let worker_id: usize = args[3].parse().unwrap();
        let cpu_iters: usize = args[4].parse().unwrap();
        let io_iters: usize = args[5].parse().unwrap();
        let io_perc: f64 = args[6].parse().unwrap();
        let buffer_kb: usize = args[7].parse().unwrap();
        let io_buffer_kb: usize = args[8].parse().unwrap();
        let intensity: f64 = args[9].parse().unwrap();
        let sleep_us: u64 = args[10].parse().unwrap();
        let max_workers: usize = args[11].parse().unwrap();
        let random_access: bool = args[12].parse().unwrap();
        let direct_io: bool = args[13].parse().unwrap();

        run_worker_process(shm_name, worker_id, cpu_iters, io_iters, io_perc, buffer_kb * 1024, io_buffer_kb * 1024, intensity, sleep_us, max_workers, random_access, direct_io);
        return;
    }

    // Hidden SoI worker subcommand — child process entry point
    if experiment == "__soi_worker" {
        // Args: __soi_worker <shm_name> <worker_id> <max_workers> <soi_type> <buffer_size>
        let shm_name = &args[2];
        let worker_id: usize = args[3].parse().unwrap();
        let max_workers: usize = args[4].parse().unwrap();
        let soi_type = soi::SoiType::from_str(&args[5]).expect("Invalid SoI type");
        let buffer_size: usize = args[6].parse().unwrap();

        soi::run_soi_worker_process(shm_name, worker_id, max_workers, soi_type, buffer_size);
        return;
    }

    // Clean up any leftover files from previous runs
    cleanup_scratch_files();

    // Parse tuning parameters from optional flags
    let params = parse_tuning_params(&args);

    println!("Calibrating operations...");
    let calibration = calibrate_operations_full(&params);
    println!("Calibration: {} CPU iterations/op ({}μs), {} I/O iterations/op ({}μs)",
             calibration.cpu_iterations, calibration.cpu_us,
             calibration.io_iterations, calibration.io_us);
    println!("Theoretical max: {:.0} CPU work-units/s, {:.0} I/O work-units/s per thread\n",
             calibration.cpu_ops_per_sec(), calibration.io_ops_per_sec());

    match experiment.as_str() {
        "find-saturation" => { run_saturation_experiment(SaturationExperiment {
            label: "CPU",
            mode: Mode::Threads,
            io_perc: 0.0,
            csv_base: "cpu_throughput_vs_threads".into(),
            recommendation: Some("find-io-slack"),
        }, calibration, &params); },
        "find-io-saturation" => { run_saturation_experiment(SaturationExperiment {
            label: "I/O",
            mode: Mode::Threads,
            io_perc: 1.0,
            csv_base: "io_throughput_vs_threads".into(),
            recommendation: Some("find-cpu-slack"),
        }, calibration, &params); },
        "find-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(100.0);
            run_slack_experiment(SlackExperiment {
                baseline_label: "CPU",
                baseline_io_ratio: 0.0,
                tracked_label: "CPU",
                track_io: false,
            }, calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-io-slack" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            run_slack_experiment(SlackExperiment {
                baseline_label: "I/O",
                baseline_io_ratio: 1.0,
                tracked_label: "I/O",
                track_io: true,
            }, calibration, &params, baseline, extra_io_pct / 100.0);
        },
        "find-saturation-proc" => {
            let result = run_saturation_experiment(SaturationExperiment {
                label: "CPU",
                mode: Mode::Procs,
                io_perc: 0.0,
                csv_base: "proc_cpu_throughput_vs_workers".into(),
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, 0, calibration, &params);
                }
            }
        },
        "find-io-saturation-proc" => {
            let result = run_saturation_experiment(SaturationExperiment {
                label: "I/O",
                mode: Mode::Procs,
                io_perc: 1.0,
                csv_base: "proc_io_throughput_vs_workers".into(),
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, 100, calibration, &params);
                }
            }
        },
        "find-mixed-saturation-proc" => {
            let io_pct = args.get(2).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-mixed-saturation-proc <io_pct> [OPTIONS]");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_perc = (io_pct.clamp(0.0, 100.0)) / 100.0;
            let io_pct_int = io_pct as u32;
            let result = run_saturation_experiment(SaturationExperiment {
                label: "Mixed",
                mode: Mode::Procs,
                io_perc,
                csv_base: format!("proc_mixed_{}pct_io_throughput_vs_workers", io_pct_int),
                recommendation: None,
            }, calibration, &params);
            if params.chain {
                if let Some((n, io_perc)) = result {
                    println!("\n=== CHAINING: intensity sweep with {} base workers ===\n", n);
                    run_intensity_sweep_experiment(n, io_perc, io_pct_int, calibration, &params);
                }
            }
        },
        "find-saturation-intensity-proc" => {
            let base_workers = args.get(2).and_then(|s| s.parse::<usize>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-saturation-intensity-proc <N> <io_pct> [OPTIONS]");
                eprintln!("  N: number of base workers at intensity=1.0");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_pct_int = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-saturation-intensity-proc <N> <io_pct> [OPTIONS]");
                eprintln!("  io_pct: IO percentage 0-100 (e.g. 50 for 50% IO)");
                std::process::exit(1);
            });
            let io_perc = (io_pct_int as f64).clamp(0.0, 100.0) / 100.0;
            run_intensity_sweep_experiment(base_workers, io_perc, io_pct_int, calibration, &params);
        },
        "find-slack-proc" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            run_slack_proc_experiment("CPU", 0.0, false, baseline, extra_io_pct / 100.0, calibration, &params);
        },
        "find-io-slack-proc" => {
            let baseline = args.get(2).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-io-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            let extra_io_pct = args.get(3).and_then(|s| s.parse::<f64>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-io-slack-proc <N> <extra_io%> [OPTIONS]");
                std::process::exit(1);
            });
            run_slack_proc_experiment("I/O", 1.0, true, baseline, extra_io_pct / 100.0, calibration, &params);
        },
        "find-soi-slack" => {
            let soi_str = args.get(2).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-soi-slack <soi|all> <victim_workers> [victim_io%] [OPTIONS]");
                eprintln!("  SoI types: l1d, l2, l3, membw, memcap, cpu, iobw, iops, all");
                std::process::exit(1);
            });
            let soi_types = soi::parse_soi_list(soi_str).unwrap_or_else(|e| {
                eprintln!("{}", e);
                std::process::exit(1);
            });
            let victim_workers = args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or_else(|| {
                eprintln!("Usage: saturator find-soi-slack <soi|all> <victim_workers> [victim_io%] [OPTIONS]");
                std::process::exit(1);
            });
            let victim_io_pct = args.get(4).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
            let victim_io_perc = victim_io_pct.clamp(0.0, 100.0) / 100.0;

            let cache_sizes = soi::detect_cache_sizes();
            println!("Detected cache sizes: L1d={}KB, L2={}KB, L3={}KB",
                     cache_sizes.l1d / 1024, cache_sizes.l2 / 1024, cache_sizes.l3 / 1024);

            run_soi_experiments(&soi_types, victim_workers, victim_io_perc, &cache_sizes, calibration, &params);
        },
        _ => println!("Unknown experiment: {}", experiment),
    }

    // Clean up after run
    cleanup_scratch_files();
}

fn parse_tuning_params(args: &[String]) -> TuningParams {
    let mut params = TuningParams::default();
    // For -proc variants, use higher default max_workers
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get()).unwrap_or(4);
    if args.len() >= 2 && (args[1].ends_with("-proc") || args[1] == "find-soi-slack") {
        params.max_workers = parallelism * 16;
    }

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--buffer-kb" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.buffer_kb = v;
                }
            }
            "--io-buffer-kb" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.io_buffer_kb = v;
                }
            }
            "--max-workers" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.max_workers = v;
                }
            }
            "--duration" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.duration_secs = v;
                }
            }
            "--samples" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    params.samples = v.max(1);
                }
            }
            "--step" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<usize>().ok()) {
                    params.step = v.max(1);
                }
            }
            "--intensity" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<f64>().ok()) {
                    params.intensity = v.clamp(0.0, 1.0);
                }
            }
            "--chain" => {
                params.chain = true;
            }
            "--random-access" => {
                params.random_access = true;
            }
            "--direct-io" => {
                params.direct_io = true;
            }
            "--warmup" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse().ok()) {
                    params.warmup_secs = v;
                }
            }
            "--sample-interval" => {
                i += 1;
                if let Some(v) = args.get(i).and_then(|s| s.parse::<u64>().ok()) {
                    params.sample_interval_ms = Some(v.max(100));
                }
            }
            _ => {}
        }
        i += 1;
    }
    params
}
