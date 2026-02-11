use std::fs::File;
use std::io::{Write, BufWriter};
use std::path::PathBuf;
use std::env;

pub struct SaturationResult {
    pub thread_count: usize,
    pub throughput: f64,
    pub throughput_per_thread: f64,
}

pub struct IoSlackResult {
    pub cpu_threads: usize,
    pub io_threads: usize,
    pub cpu_ops: f64,
    pub io_ops: f64,
    pub cpu_degradation_pct: f64,
}

pub struct ResultsWriter {
    saturation_results: Vec<SaturationResult>,
    io_slack_results: Vec<IoSlackResult>,
    baseline_cpu_throughput: Option<f64>,
}

impl ResultsWriter {
    pub fn new() -> Self {
        Self {
            saturation_results: Vec::new(),
            io_slack_results: Vec::new(),
            baseline_cpu_throughput: None,
        }
    }

    pub fn add_saturation_point(&mut self, thread_count: usize, throughput: f64) {
        self.saturation_results.push(SaturationResult {
            thread_count,
            throughput,
            throughput_per_thread: throughput / thread_count as f64,
        });
    }

    pub fn set_baseline(&mut self, throughput: f64) {
        self.baseline_cpu_throughput = Some(throughput);
    }

    pub fn add_io_slack_point(&mut self, cpu_threads: usize, io_threads: usize, cpu_ops: f64, io_ops: f64) {
        let baseline = self.baseline_cpu_throughput.unwrap_or(cpu_ops);
        let degradation = (baseline - cpu_ops) / baseline * 100.0;
        
        self.io_slack_results.push(IoSlackResult {
            cpu_threads,
            io_threads,
            cpu_ops,
            io_ops,
            cpu_degradation_pct: degradation,
        });
    }

    pub fn write_saturation_csv(&self, filename: &str) -> std::io::Result<PathBuf> {
        let path = env::current_dir()?.join(filename);
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        
        writeln!(writer, "threads,throughput_ops_sec,throughput_per_thread")?;
        
        for r in &self.saturation_results {
            writeln!(writer, "{},{:.2},{:.2}", 
                     r.thread_count, r.throughput, r.throughput_per_thread)?;
        }
        
        writer.flush()?;
        println!("Saturation results written to: {}", path.display());
        Ok(path)
    }

    pub fn write_io_slack_csv(&self, filename: &str) -> std::io::Result<PathBuf> {
        let path = env::current_dir()?.join(filename);
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        
        writeln!(writer, "cpu_threads,io_threads,total_threads,cpu_ops_sec,io_ops_sec,cpu_degradation_pct")?;
        
        // Write baseline if available
        if let Some(baseline) = self.baseline_cpu_throughput {
            if let Some(first) = self.io_slack_results.first() {
                writeln!(writer, "{},0,{},{:.2},0,0.00", 
                         first.cpu_threads, first.cpu_threads, baseline)?;
            }
        }
        
        for r in &self.io_slack_results {
            writeln!(writer, "{},{},{},{:.2},{:.2},{:.2}", 
                     r.cpu_threads, r.io_threads, r.cpu_threads + r.io_threads,
                     r.cpu_ops, r.io_ops, r.cpu_degradation_pct)?;
        }
        
        writer.flush()?;
        println!("I/O slack results written to: {}", path.display());
        Ok(path)
    }
}
