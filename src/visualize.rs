use std::fs::File;
use std::io::{Write, BufWriter};

use crate::proc_metrics::{self, SystemMetrics};

/// A single data point in a saturation experiment.
pub struct SaturationResult {
    pub thread_count: usize,
    pub cpu_ops: f64,
    pub io_ops: f64,
    pub total_ops: f64,
    pub throughput_per_thread: f64,
    pub cpu_ops_stddev: f64,
    pub io_ops_stddev: f64,
    pub metrics: SystemMetrics,
}

/// Collects saturation data points and writes them to CSV.
pub struct ResultsWriter {
    saturation_results: Vec<SaturationResult>,
}

impl ResultsWriter {
    pub fn new() -> Self {
        Self {
            saturation_results: Vec::new(),
        }
    }

    /// Record a data point: worker count, throughput values, stddevs, and system metrics.
    pub fn add_saturation_point(&mut self, thread_count: usize, cpu_ops: f64, io_ops: f64, cpu_ops_stddev: f64, io_ops_stddev: f64, metrics: SystemMetrics) {
        let total_ops = cpu_ops + io_ops;
        self.saturation_results.push(SaturationResult {
            thread_count,
            cpu_ops,
            io_ops,
            total_ops,
            throughput_per_thread: total_ops / thread_count as f64,
            cpu_ops_stddev,
            io_ops_stddev,
            metrics,
        });
    }

    /// Write all collected data points to a CSV file at the given path.
    pub fn write_saturation_csv(&self, path: &std::path::Path) -> std::io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        writeln!(writer, "threads,cpu_ops_sec,io_ops_sec,total_ops_sec,throughput_per_thread,cpu_ops_stddev,io_ops_stddev,{}",
                 proc_metrics::csv_header())?;

        for r in &self.saturation_results {
            writeln!(writer, "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}",
                     r.thread_count, r.cpu_ops, r.io_ops, r.total_ops,
                     r.throughput_per_thread,
                     r.cpu_ops_stddev, r.io_ops_stddev,
                     r.metrics.to_csv_row())?;
        }

        writer.flush()?;
        println!("Results written to: {}", path.display());
        Ok(())
    }
}
