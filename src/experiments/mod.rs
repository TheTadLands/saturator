pub mod saturation;
pub mod slack;
pub mod intensity;
pub mod slack_proc;

pub use saturation::run_saturation_experiment;
pub use slack::run_slack_experiment;
pub use intensity::run_intensity_sweep_experiment;
pub use slack_proc::run_slack_proc_experiment;

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
    pub csv_base: &'static str,
    pub recommendation: Option<&'static str>,
}

/// Configuration for a thread-based slack experiment (baseline + extra threads).
pub struct SlackExperiment {
    pub baseline_label: &'static str,
    pub baseline_io_ratio: f64,
    pub tracked_label: &'static str,
    pub track_io: bool,
}
