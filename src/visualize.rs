use std::fs::File;
use std::io::{Write, BufWriter};
use std::path::PathBuf;
use std::env;

pub struct SaturationResult {
    pub thread_count: usize,
    pub throughput: f64,
    pub throughput_per_thread: f64,
}

pub struct ResultsWriter {
    saturation_results: Vec<SaturationResult>,
}

impl ResultsWriter {
    pub fn new() -> Self {
        Self {
            saturation_results: Vec::new(),
        }
    }

    pub fn add_saturation_point(&mut self, thread_count: usize, throughput: f64) {
        self.saturation_results.push(SaturationResult {
            thread_count,
            throughput,
            throughput_per_thread: throughput / thread_count as f64,
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
}
