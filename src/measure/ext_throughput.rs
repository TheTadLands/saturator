/// Reader for the external workload throughput protocol file.
///
/// Protocol: one line per sample, format: `<timestamp_ms> <ops_per_sec>\n`
/// The external workload (or its wrapper script) appends lines to this file.
/// This reader tracks how many lines have been consumed, returning only new ones.
pub struct ThroughputReader {
    path: String,
    last_line_count: usize,
}

impl ThroughputReader {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            last_line_count: 0,
        }
    }

    /// Read any new lines since the last call.
    /// Returns Vec of (timestamp_ms, ops_per_sec) for each new line.
    /// Returns empty vec if the file doesn't exist, is empty, or has no new lines.
    pub fn read_new_samples(&mut self) -> Vec<(u64, f64)> {
        let contents = match std::fs::read_to_string(&self.path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let lines: Vec<&str> = contents.lines().collect();
        if lines.len() <= self.last_line_count {
            return Vec::new();
        }

        let new_lines = &lines[self.last_line_count..];
        let mut samples = Vec::with_capacity(new_lines.len());

        for line in new_lines {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(ts), Ok(ops)) = (parts[0].parse::<u64>(), parts[1].parse::<f64>()) {
                    samples.push((ts, ops));
                }
            }
        }

        self.last_line_count = lines.len();
        samples
    }

    /// Compute the average ops/sec from a set of samples.
    pub fn average_ops(samples: &[(u64, f64)]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        samples.iter().map(|(_, ops)| ops).sum::<f64>() / samples.len() as f64
    }

    /// Reset the reader, discarding knowledge of previously read lines.
    /// Call after warmup to start fresh for the measurement window.
    pub fn reset(&mut self) {
        // Read current line count so we skip everything written during warmup
        if let Ok(contents) = std::fs::read_to_string(&self.path) {
            self.last_line_count = contents.lines().count();
        } else {
            self.last_line_count = 0;
        }
    }
}
