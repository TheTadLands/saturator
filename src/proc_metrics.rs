use std::fs::read_to_string;

pub struct ProcSnapshot {
    // Cgroup CPU accounting (microseconds)
    usage_usec: u64,
    system_usec: u64,
    // Number of CPUs available to this cgroup
    num_cpus: usize,
    // IO bytes from cgroup io.stat (summed across all devices)
    io_rbytes: u64,
    io_wbytes: u64,
    // IO bandwidth limit from cgroup io.max (bytes/sec, summed read+write across devices)
    // None if no limit is configured
    io_max_bps: Option<u64>,
    // PSI cumulative microseconds (cgroup-scoped)
    psi_cpu_some_us: u64,
    psi_io_some_us: u64,
    psi_io_full_us: u64,
}

#[derive(Clone, Debug)]
pub struct SystemMetrics {
    pub cpu_pct: f64,
    pub system_pct: f64,
    pub io_util_pct: f64,
    pub io_psi_pct: f64,
    pub psi_cpu_some_us: u64,
    pub psi_io_some_us: u64,
    pub psi_io_full_us: u64,
}

pub fn take_snapshot() -> ProcSnapshot {
    let (usage_usec, system_usec) = read_cgroup_cpu_stat();

    let num_cpus = read_cpuset_cpus()
        .unwrap_or_else(|| count_proc_stat_cpus());

    let (io_rbytes, io_wbytes) = read_io_stat();
    let io_max_bps = read_io_max();

    // PSI: prefer cgroup-scoped, fall back to system-wide
    let psi_cpu_some_us = read_psi_total("/sys/fs/cgroup/cpu.pressure", "some")
        .or_else(|| read_psi_total("/proc/pressure/cpu", "some"))
        .unwrap_or(0);
    let psi_io_some_us = read_psi_total("/sys/fs/cgroup/io.pressure", "some")
        .or_else(|| read_psi_total("/proc/pressure/io", "some"))
        .unwrap_or(0);
    let psi_io_full_us = read_psi_total("/sys/fs/cgroup/io.pressure", "full")
        .or_else(|| read_psi_total("/proc/pressure/io", "full"))
        .unwrap_or(0);

    ProcSnapshot {
        usage_usec, system_usec, num_cpus,
        io_rbytes, io_wbytes, io_max_bps,
        psi_cpu_some_us, psi_io_some_us, psi_io_full_us,
    }
}

fn read_cgroup_cpu_stat() -> (u64, u64) {
    let content = read_to_string("/sys/fs/cgroup/cpu.stat").unwrap_or_default();
    let mut usage = 0u64;
    let mut system = 0u64;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("usage_usec") => usage = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            Some("system_usec") => system = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            _ => {}
        }
    }
    (usage, system)
}

/// Parse cpuset.cpus.effective or cpuset.cpus (e.g. "4-7" or "0,2,4-6")
fn read_cpuset_cpus() -> Option<usize> {
    let content = read_to_string("/sys/fs/cgroup/cpuset.cpus.effective")
        .or_else(|_| read_to_string("/sys/fs/cgroup/cpuset.cpus"))
        .ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut count = 0usize;
    for part in trimmed.split(',') {
        if let Some((lo, hi)) = part.split_once('-') {
            let lo: usize = lo.parse().ok()?;
            let hi: usize = hi.parse().ok()?;
            count += hi - lo + 1;
        } else {
            let _: usize = part.parse().ok()?;
            count += 1;
        }
    }
    Some(count)
}

/// Count per-CPU lines in /proc/stat as fallback
fn count_proc_stat_cpus() -> usize {
    let stat = read_to_string("/proc/stat").unwrap_or_default();
    let count = stat.lines()
        .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
        .count();
    if count > 0 { count } else { 1 }
}

/// Read cumulative IO bytes from cgroup io.stat, summed across all devices.
/// Format: "MAJ:MIN rbytes=N wbytes=N rios=N wios=N dbytes=N dios=N"
fn read_io_stat() -> (u64, u64) {
    let content = read_to_string("/sys/fs/cgroup/io.stat").unwrap_or_default();
    let mut total_rbytes = 0u64;
    let mut total_wbytes = 0u64;
    for line in content.lines() {
        for part in line.split_whitespace() {
            if let Some(val) = part.strip_prefix("rbytes=") {
                total_rbytes += val.parse::<u64>().unwrap_or(0);
            } else if let Some(val) = part.strip_prefix("wbytes=") {
                total_wbytes += val.parse::<u64>().unwrap_or(0);
            }
        }
    }
    (total_rbytes, total_wbytes)
}

/// Read IO bandwidth limit from cgroup io.max.
/// Format: "MAJ:MIN rbps=N wbps=N riops=N wiops=N"
/// Returns the minimum of all configured rbps+wbps limits (uses the tightest constraint).
/// "max" means unlimited for that field.
fn read_io_max() -> Option<u64> {
    let content = read_to_string("/sys/fs/cgroup/io.max").ok()?;
    let mut limit: Option<u64> = None;
    for line in content.lines() {
        let mut rbps: Option<u64> = None;
        let mut wbps: Option<u64> = None;
        for part in line.split_whitespace() {
            if let Some(val) = part.strip_prefix("rbps=") {
                if val != "max" {
                    rbps = val.parse().ok();
                }
            } else if let Some(val) = part.strip_prefix("wbps=") {
                if val != "max" {
                    wbps = val.parse().ok();
                }
            }
        }
        // Use the average of read+write limits if both are set,
        // or whichever one is set, as the effective bandwidth cap
        let device_limit = match (rbps, wbps) {
            (Some(r), Some(w)) => Some((r + w) / 2),
            (Some(r), None) => Some(r),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };
        if let Some(dl) = device_limit {
            limit = Some(limit.map_or(dl, |l: u64| l.min(dl)));
        }
    }
    limit
}

fn read_psi_total(path: &str, kind: &str) -> Option<u64> {
    let content = read_to_string(path).ok()?;
    for line in content.lines() {
        if line.starts_with(kind) {
            // Format: "some avg10=0.00 avg60=0.00 avg300=0.00 total=12345"
            for part in line.split_whitespace() {
                if let Some(val) = part.strip_prefix("total=") {
                    return val.parse().ok();
                }
            }
        }
    }
    None
}

pub fn compute_delta(before: &ProcSnapshot, after: &ProcSnapshot, duration_secs: f64) -> SystemMetrics {
    let d_usage = after.usage_usec.saturating_sub(before.usage_usec);
    let d_system = after.system_usec.saturating_sub(before.system_usec);

    // Total available CPU-time in the measurement window (microseconds)
    let total_available_us = duration_secs * 1_000_000.0 * after.num_cpus as f64;

    let cpu_pct = if total_available_us > 0.0 {
        d_usage as f64 / total_available_us * 100.0
    } else {
        0.0
    };
    let system_pct = if total_available_us > 0.0 {
        d_system as f64 / total_available_us * 100.0
    } else {
        0.0
    };

    // IO bandwidth utilization: delta bytes / duration / max_bps
    let d_rbytes = after.io_rbytes.saturating_sub(before.io_rbytes);
    let d_wbytes = after.io_wbytes.saturating_sub(before.io_wbytes);
    let d_bytes = d_rbytes + d_wbytes;
    let io_util_pct = if let Some(max_bps) = after.io_max_bps {
        if max_bps > 0 && duration_secs > 0.0 {
            let actual_bps = d_bytes as f64 / duration_secs;
            (actual_bps / max_bps as f64 * 100.0).min(100.0)
        } else {
            0.0
        }
    } else {
        0.0
    };

    let d_psi_io_some = after.psi_io_some_us.saturating_sub(before.psi_io_some_us);
    let wall_us = duration_secs * 1_000_000.0;
    let io_psi_pct = if wall_us > 0.0 {
        d_psi_io_some as f64 / wall_us * 100.0
    } else {
        0.0
    };

    SystemMetrics {
        cpu_pct,
        system_pct,
        io_util_pct,
        io_psi_pct,
        psi_cpu_some_us: after.psi_cpu_some_us.saturating_sub(before.psi_cpu_some_us),
        psi_io_some_us: d_psi_io_some,
        psi_io_full_us: after.psi_io_full_us.saturating_sub(before.psi_io_full_us),
    }
}

pub fn csv_header() -> &'static str {
    "cpu_pct,system_pct,io_util_pct,io_psi_pct,psi_cpu_some_us,psi_io_some_us,psi_io_full_us"
}

impl SystemMetrics {
    pub fn to_csv_row(&self) -> String {
        format!("{:.2},{:.2},{:.2},{:.2},{},{},{}",
            self.cpu_pct, self.system_pct, self.io_util_pct, self.io_psi_pct,
            self.psi_cpu_some_us, self.psi_io_some_us, self.psi_io_full_us)
    }
}

pub fn median_metrics(samples: &[SystemMetrics]) -> SystemMetrics {
    if samples.is_empty() {
        return empty();
    }
    if samples.len() == 1 {
        return samples[0].clone();
    }

    fn median_f64(vals: &mut Vec<f64>) -> f64 {
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        vals[vals.len() / 2]
    }
    fn median_u64(vals: &mut Vec<u64>) -> u64 {
        vals.sort();
        vals[vals.len() / 2]
    }

    let mut cpu_pct: Vec<f64> = samples.iter().map(|s| s.cpu_pct).collect();
    let mut system_pct: Vec<f64> = samples.iter().map(|s| s.system_pct).collect();
    let mut io_util_pct: Vec<f64> = samples.iter().map(|s| s.io_util_pct).collect();
    let mut io_psi_pct: Vec<f64> = samples.iter().map(|s| s.io_psi_pct).collect();
    let mut psi_cpu_some_us: Vec<u64> = samples.iter().map(|s| s.psi_cpu_some_us).collect();
    let mut psi_io_some_us: Vec<u64> = samples.iter().map(|s| s.psi_io_some_us).collect();
    let mut psi_io_full_us: Vec<u64> = samples.iter().map(|s| s.psi_io_full_us).collect();

    SystemMetrics {
        cpu_pct: median_f64(&mut cpu_pct),
        system_pct: median_f64(&mut system_pct),
        io_util_pct: median_f64(&mut io_util_pct),
        io_psi_pct: median_f64(&mut io_psi_pct),
        psi_cpu_some_us: median_u64(&mut psi_cpu_some_us),
        psi_io_some_us: median_u64(&mut psi_io_some_us),
        psi_io_full_us: median_u64(&mut psi_io_full_us),
    }
}

pub fn empty() -> SystemMetrics {
    SystemMetrics {
        cpu_pct: 0.0,
        system_pct: 0.0,
        io_util_pct: 0.0,
        io_psi_pct: 0.0,
        psi_cpu_some_us: 0,
        psi_io_some_us: 0,
        psi_io_full_us: 0,
    }
}
