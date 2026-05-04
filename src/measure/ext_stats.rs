/// Reader for the external workload stats protocol file.
///
/// Protocol: narrow CSV, one row per observation, format:
///   `<timestamp_ms>,<metric_name>,<value>\n`
///
/// Workload-agnostic: the wrapper chooses which metrics to emit and at what
/// cadence (periodic time-series, end-of-run aggregate, or a mix). Saturator
/// reads all rows written during a measurement run and persists them verbatim
/// in the experiment's stats CSV. Metric names are free-form strings; values
/// must parse as f64. Rows that don't parse are silently skipped so malformed
/// output from a workload doesn't abort the sweep.
pub struct StatsRow {
    pub timestamp_ms: u64,
    pub metric: String,
    pub value: f64,
}

/// Truncate the stats file so stats from a previous measurement don't bleed
/// into the next one. Does nothing if the file doesn't exist.
pub fn truncate(path: &str) {
    let _ = std::fs::write(path, "");
}

/// Read every row from the stats file, returning parsed observations. Empty
/// or missing files yield an empty vec. Malformed rows are dropped.
pub fn read_all(path: &str) -> Vec<StatsRow> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut rows = Vec::new();
    for line in contents.lines() {
        let parts: Vec<&str> = line.splitn(3, ',').collect();
        if parts.len() != 3 {
            continue;
        }
        let ts = match parts[0].trim().parse::<u64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let metric = parts[1].trim().to_string();
        if metric.is_empty() {
            continue;
        }
        let value = match parts[2].trim().parse::<f64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        rows.push(StatsRow { timestamp_ms: ts, metric, value });
    }
    rows
}
