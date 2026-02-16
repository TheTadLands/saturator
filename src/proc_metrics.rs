use std::fs::read_to_string;

pub struct ProcSnapshot {
    // CPU jiffies from /proc/stat first line
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    steal: u64,
    // Context switches from /proc/stat ctxt line
    context_switches: u64,
    // PSI cumulative microseconds
    psi_cpu_some_us: u64,
    psi_io_some_us: u64,
    psi_io_full_us: u64,
}

#[derive(Clone, Debug)]
pub struct SystemMetrics {
    pub cpu_pct: f64,
    pub iowait_pct: f64,
    pub system_pct: f64,
    pub steal_pct: f64,
    pub context_switches: u64,
    pub ctx_switches_per_sec: f64,
    pub psi_cpu_some_us: u64,
    pub psi_io_some_us: u64,
    pub psi_io_full_us: u64,
}

pub fn take_snapshot() -> ProcSnapshot {
    let stat = read_to_string("/proc/stat").unwrap_or_default();
    let mut user = 0u64;
    let mut nice = 0u64;
    let mut system = 0u64;
    let mut idle = 0u64;
    let mut iowait = 0u64;
    let mut irq = 0u64;
    let mut softirq = 0u64;
    let mut steal = 0u64;
    let mut context_switches = 0u64;

    for line in stat.lines() {
        if line.starts_with("cpu ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 9 {
                user = parts[1].parse().unwrap_or(0);
                nice = parts[2].parse().unwrap_or(0);
                system = parts[3].parse().unwrap_or(0);
                idle = parts[4].parse().unwrap_or(0);
                iowait = parts[5].parse().unwrap_or(0);
                irq = parts[6].parse().unwrap_or(0);
                softirq = parts[7].parse().unwrap_or(0);
                steal = parts[8].parse().unwrap_or(0);
            }
        } else if line.starts_with("ctxt ") {
            context_switches = line.split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        }
    }

    let psi_cpu_some_us = read_psi_total("/proc/pressure/cpu", "some");
    let psi_io_some_us = read_psi_total("/proc/pressure/io", "some");
    let psi_io_full_us = read_psi_total("/proc/pressure/io", "full");

    ProcSnapshot {
        user, nice, system, idle, iowait, irq, softirq, steal,
        context_switches,
        psi_cpu_some_us, psi_io_some_us, psi_io_full_us,
    }
}

fn read_psi_total(path: &str, kind: &str) -> u64 {
    let content = match read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    for line in content.lines() {
        if line.starts_with(kind) {
            // Format: "some avg10=0.00 avg60=0.00 avg300=0.00 total=12345"
            for part in line.split_whitespace() {
                if let Some(val) = part.strip_prefix("total=") {
                    return val.parse().unwrap_or(0);
                }
            }
        }
    }
    0
}

pub fn compute_delta(before: &ProcSnapshot, after: &ProcSnapshot, duration_secs: f64) -> SystemMetrics {
    let d_user = after.user.saturating_sub(before.user);
    let d_nice = after.nice.saturating_sub(before.nice);
    let d_system = after.system.saturating_sub(before.system);
    let d_idle = after.idle.saturating_sub(before.idle);
    let d_iowait = after.iowait.saturating_sub(before.iowait);
    let d_irq = after.irq.saturating_sub(before.irq);
    let d_softirq = after.softirq.saturating_sub(before.softirq);
    let d_steal = after.steal.saturating_sub(before.steal);

    let total = d_user + d_nice + d_system + d_idle + d_iowait + d_irq + d_softirq + d_steal;
    let total_f = if total == 0 { 1.0 } else { total as f64 };

    let ctx = after.context_switches.saturating_sub(before.context_switches);

    SystemMetrics {
        cpu_pct: (d_user + d_nice + d_system) as f64 / total_f * 100.0,
        iowait_pct: d_iowait as f64 / total_f * 100.0,
        system_pct: d_system as f64 / total_f * 100.0,
        steal_pct: d_steal as f64 / total_f * 100.0,
        context_switches: ctx,
        ctx_switches_per_sec: if duration_secs > 0.0 { ctx as f64 / duration_secs } else { 0.0 },
        psi_cpu_some_us: after.psi_cpu_some_us.saturating_sub(before.psi_cpu_some_us),
        psi_io_some_us: after.psi_io_some_us.saturating_sub(before.psi_io_some_us),
        psi_io_full_us: after.psi_io_full_us.saturating_sub(before.psi_io_full_us),
    }
}

pub fn csv_header() -> &'static str {
    "cpu_pct,iowait_pct,system_pct,steal_pct,context_switches,ctx_switches_per_sec,psi_cpu_some_us,psi_io_some_us,psi_io_full_us"
}

impl SystemMetrics {
    pub fn to_csv_row(&self) -> String {
        format!("{:.2},{:.2},{:.2},{:.2},{},{:.0},{},{},{}",
            self.cpu_pct, self.iowait_pct, self.system_pct, self.steal_pct,
            self.context_switches, self.ctx_switches_per_sec,
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
    let mut iowait_pct: Vec<f64> = samples.iter().map(|s| s.iowait_pct).collect();
    let mut system_pct: Vec<f64> = samples.iter().map(|s| s.system_pct).collect();
    let mut steal_pct: Vec<f64> = samples.iter().map(|s| s.steal_pct).collect();
    let mut context_switches: Vec<u64> = samples.iter().map(|s| s.context_switches).collect();
    let mut ctx_switches_per_sec: Vec<f64> = samples.iter().map(|s| s.ctx_switches_per_sec).collect();
    let mut psi_cpu_some_us: Vec<u64> = samples.iter().map(|s| s.psi_cpu_some_us).collect();
    let mut psi_io_some_us: Vec<u64> = samples.iter().map(|s| s.psi_io_some_us).collect();
    let mut psi_io_full_us: Vec<u64> = samples.iter().map(|s| s.psi_io_full_us).collect();

    SystemMetrics {
        cpu_pct: median_f64(&mut cpu_pct),
        iowait_pct: median_f64(&mut iowait_pct),
        system_pct: median_f64(&mut system_pct),
        steal_pct: median_f64(&mut steal_pct),
        context_switches: median_u64(&mut context_switches),
        ctx_switches_per_sec: median_f64(&mut ctx_switches_per_sec),
        psi_cpu_some_us: median_u64(&mut psi_cpu_some_us),
        psi_io_some_us: median_u64(&mut psi_io_some_us),
        psi_io_full_us: median_u64(&mut psi_io_full_us),
    }
}

pub fn empty() -> SystemMetrics {
    SystemMetrics {
        cpu_pct: 0.0,
        iowait_pct: 0.0,
        system_pct: 0.0,
        steal_pct: 0.0,
        context_switches: 0,
        ctx_switches_per_sec: 0.0,
        psi_cpu_some_us: 0,
        psi_io_some_us: 0,
        psi_io_full_us: 0,
    }
}
