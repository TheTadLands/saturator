pub mod saturation;
pub mod slack;
pub mod intensity;
pub mod slack_proc;
pub mod soi_sweep;
pub mod soi_sweep_ext;

pub use saturation::run_saturation_experiment;
pub use slack::run_slack_experiment;
pub use intensity::run_intensity_sweep_experiment;
pub use slack_proc::run_slack_proc_experiment;
pub use soi_sweep::run_soi_experiments;
pub use soi_sweep_ext::{run_soi_ext_experiments, run_ext_saturation_and_sweep};

/// Whether the experiment uses threads or child processes.
#[derive(Clone, Copy)]
pub enum Mode {
    Threads,
    Procs,
}

/// Configuration for a saturation experiment (find the throughput ceiling).
pub struct SaturationExperiment {
    pub label: &'static str,
    pub mode: Mode,
    pub io_perc: f64,
    pub csv_base: String,
    pub recommendation: Option<&'static str>,
}

/// Configuration for a thread-based slack experiment (baseline + extra threads).
pub struct SlackExperiment {
    pub baseline_label: &'static str,
    pub baseline_io_ratio: f64,
    pub tracked_label: &'static str,
    pub track_io: bool,
}
