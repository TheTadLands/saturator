/// Reader for the external workload throughput protocol file.
///
/// Protocol: one line per sample, format: `<timestamp_ms> <cumulative_ops>\n`
/// The external workload (or its wrapper script) appends lines to this file.
/// This reader tracks cumulative state and computes instantaneous rates from deltas.
pub struct ThroughputReader {
    path: String,
    last_line_count: usize,
    /// Last known cumulative ops and timestamp, used to compute rates.
    last_cumulative: Option<(u64, f64)>,
    /// First cumulative seen after the most recent `reset()`. Used to compute
    /// total ops completed over the measurement window.
    first_cumulative: Option<(u64, f64)>,
}

impl ThroughputReader {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            last_line_count: 0,
            last_cumulative: None,
            first_cumulative: None,
        }
    }

    /// Total ops and elapsed seconds between the first sample after `reset()`
    /// and the most recent sample. Returns `None` until at least two samples
    /// have been seen (including the one seeded by `reset()`).
    pub fn total_ops(&self) -> Option<(f64, f64)> {
        let (first_ts, first_ops) = self.first_cumulative?;
        let (last_ts, last_ops) = self.last_cumulative?;
        let dt = last_ts.saturating_sub(first_ts) as f64 / 1000.0;
        if dt <= 0.0 {
            return None;
        }
        Some((last_ops - first_ops, dt))
    }

    /// Return the given quantiles (0.0–1.0) of a set of rate samples. Linear
    /// interpolation between sorted samples. Empty input yields zeros.
    pub fn percentiles(samples: &[(u64, f64)], qs: &[f64]) -> Vec<f64> {
        if samples.is_empty() {
            return qs.iter().map(|_| 0.0).collect();
        }
        let mut rates: Vec<f64> = samples.iter().map(|(_, r)| *r).collect();
        rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        qs.iter().map(|q| {
            let q = q.clamp(0.0, 1.0);
            let pos = q * (rates.len() - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = pos.ceil() as usize;
            if lo == hi {
                rates[lo]
            } else {
                let frac = pos - lo as f64;
                rates[lo] * (1.0 - frac) + rates[hi] * frac
            }
        }).collect()
    }

    /// Read any new lines since the last call and compute instantaneous rates
    /// from cumulative deltas.
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
                if let (Ok(ts), Ok(cum_ops)) = (parts[0].parse::<u64>(), parts[1].parse::<f64>()) {
                    if let Some((prev_ts, prev_ops)) = self.last_cumulative {
                        let dt_sec = (ts.saturating_sub(prev_ts)) as f64 / 1000.0;
                        if dt_sec > 0.0 {
                            let rate = (cum_ops - prev_ops) / dt_sec;
                            samples.push((ts, rate));
                        }
                    }
                    if self.first_cumulative.is_none() {
                        self.first_cumulative = Some((ts, cum_ops));
                    }
                    self.last_cumulative = Some((ts, cum_ops));
                }
            }
        }

        self.last_line_count = lines.len();
        samples
    }

    /// Compute the average ops/sec from a set of rate samples.
    /// Uses time-weighted averaging: each rate is weighted by the interval it
    /// covers (derived from consecutive timestamps). Falls back to simple
    /// averaging if timestamps are unavailable or degenerate.
    pub fn average_ops(samples: &[(u64, f64)]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        if samples.len() == 1 {
            return samples[0].1;
        }
        // Each sample's rate covers the interval from the previous timestamp to
        // this one. Compute total_ops = sum(rate_i * dt_i), total_time = sum(dt_i).
        let mut total_ops = 0.0;
        let mut total_time = 0.0;
        for i in 1..samples.len() {
            let dt = (samples[i].0.saturating_sub(samples[i - 1].0)) as f64 / 1000.0;
            if dt > 0.0 {
                total_ops += samples[i].1 * dt;
                total_time += dt;
            }
        }
        if total_time > 0.0 {
            total_ops / total_time
        } else {
            // Degenerate: all same timestamp, fall back to simple average
            samples.iter().map(|(_, ops)| ops).sum::<f64>() / samples.len() as f64
        }
    }

    /// Reset the reader, discarding knowledge of previously read lines.
    /// Call after warmup to start fresh for the measurement window.
    pub fn reset(&mut self) {
        // Read current line count and last cumulative value so we skip warmup data
        self.first_cumulative = None;
        if let Ok(contents) = std::fs::read_to_string(&self.path) {
            let lines: Vec<&str> = contents.lines().collect();
            self.last_line_count = lines.len();
            // Seed cumulative state from the last warmup line so the first
            // measurement sample can compute a valid delta. This seed also
            // anchors the "first sample" used by total_ops().
            if let Some(last_line) = lines.last() {
                let parts: Vec<&str> = last_line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let (Ok(ts), Ok(cum_ops)) = (parts[0].parse::<u64>(), parts[1].parse::<f64>()) {
                        self.last_cumulative = Some((ts, cum_ops));
                        self.first_cumulative = Some((ts, cum_ops));
                    }
                }
            }
        } else {
            self.last_line_count = 0;
            self.last_cumulative = None;
        }
    }
}
