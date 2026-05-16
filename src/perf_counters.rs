use std::sync::Mutex;

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_TYPE_HW_CACHE: u32 = 3;
const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;
const PERF_COUNT_HW_CACHE_MISSES: u64 = 3;

// L1-dcache-load-misses: L1D=0 | OP_READ=0<<8 | RESULT_MISS=1<<16
const L1D_READ_MISS: u64 = 0x10000;
// LLC-load-misses: LL=2 | OP_READ=0<<8 | RESULT_MISS=1<<16
const LLC_READ_MISS: u64 = 0x10002;

const PERF_FLAG_FD_CGROUP: u64 = 4;

struct PerfEvent {
    name: &'static str,
    type_: u32,
    config: u64,
}

const EVENTS: &[PerfEvent] = &[
    PerfEvent { name: "l1d_load_misses", type_: PERF_TYPE_HW_CACHE, config: L1D_READ_MISS },
    PerfEvent { name: "llc_load_misses", type_: PERF_TYPE_HW_CACHE, config: LLC_READ_MISS },
    PerfEvent { name: "cache_misses", type_: PERF_TYPE_HARDWARE, config: PERF_COUNT_HW_CACHE_MISSES },
    PerfEvent { name: "instructions", type_: PERF_TYPE_HARDWARE, config: PERF_COUNT_HW_INSTRUCTIONS },
    PerfEvent { name: "cycles", type_: PERF_TYPE_HARDWARE, config: PERF_COUNT_HW_CPU_CYCLES },
];

#[repr(C)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    config2: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    sample_max_stack: u16,
    __reserved_2: u16,
    aux_sample_size: u32,
    __reserved_3: u32,
}

#[derive(Clone, Debug)]
pub struct PerfSnapshot {
    pub l1d_load_misses: u64,
    pub llc_load_misses: u64,
    pub cache_misses: u64,
    pub instructions: u64,
    pub cycles: u64,
}

#[derive(Clone, Debug)]
pub struct PerfMetrics {
    pub l1d_load_misses: u64,
    pub llc_load_misses: u64,
    pub cache_misses: u64,
    pub instructions: u64,
    pub cycles: u64,
    pub ipc: f64,
    pub l1d_miss_per_kinsn: f64,
    pub llc_miss_per_kinsn: f64,
}

struct CounterGroup {
    fds: Vec<Vec<i32>>,
}

// SAFETY: fds are plain integers, safe to share across threads
unsafe impl Send for CounterGroup {}

static COUNTERS: Mutex<Option<CounterGroup>> = Mutex::new(None);

pub fn is_open() -> bool {
    COUNTERS.lock().unwrap().is_some()
}

pub fn open() -> bool {
    let cpus = read_cpuset_cpu_list();
    if cpus.is_empty() {
        eprintln!("perf: could not determine CPU list, skipping");
        return false;
    }

    let cgroup_fd = unsafe {
        libc::open(b"/sys/fs/cgroup\0".as_ptr() as *const libc::c_char, libc::O_RDONLY)
    };
    if cgroup_fd < 0 {
        eprintln!("perf: could not open /sys/fs/cgroup ({}), skipping",
                  std::io::Error::last_os_error());
        return false;
    }

    let mut all_fds: Vec<Vec<i32>> = Vec::with_capacity(EVENTS.len());
    let mut opened_names: Vec<&str> = Vec::new();

    for event in EVENTS {
        let mut attr = unsafe { std::mem::zeroed::<PerfEventAttr>() };
        attr.type_ = event.type_;
        attr.size = std::mem::size_of::<PerfEventAttr>() as u32;
        attr.config = event.config;

        let mut cpu_fds = Vec::with_capacity(cpus.len());
        let mut ok = true;

        for &cpu in &cpus {
            let fd = unsafe {
                libc::syscall(
                    libc::SYS_perf_event_open,
                    &attr as *const PerfEventAttr,
                    cgroup_fd,
                    cpu as libc::c_int,
                    -1 as libc::c_int,
                    PERF_FLAG_FD_CGROUP,
                )
            } as i32;

            if fd < 0 {
                eprintln!("perf: {} on CPU {} failed ({}), skipping event",
                          event.name, cpu, std::io::Error::last_os_error());
                for &prev_fd in &cpu_fds {
                    unsafe { libc::close(prev_fd); }
                }
                ok = false;
                break;
            }
            cpu_fds.push(fd);
        }

        if ok && !cpu_fds.is_empty() {
            opened_names.push(event.name);
            all_fds.push(cpu_fds);
        } else {
            all_fds.push(Vec::new());
        }
    }

    unsafe { libc::close(cgroup_fd); }

    if opened_names.is_empty() {
        eprintln!("perf: no hardware counters available");
        return false;
    }

    println!("perf: enabled counters: {} ({} CPUs)", opened_names.join(", "), cpus.len());
    *COUNTERS.lock().unwrap() = Some(CounterGroup { fds: all_fds });
    true
}

pub fn read_snapshot() -> Option<PerfSnapshot> {
    let guard = COUNTERS.lock().unwrap();
    let group = guard.as_ref()?;

    let mut values = [0u64; 5];
    for (i, fds) in group.fds.iter().enumerate() {
        let mut total = 0u64;
        for &fd in fds {
            let mut val = 0u64;
            let ret = unsafe {
                libc::read(fd, &mut val as *mut u64 as *mut libc::c_void, 8)
            };
            if ret == 8 {
                total += val;
            }
        }
        values[i] = total;
    }

    Some(PerfSnapshot {
        l1d_load_misses: values[0],
        llc_load_misses: values[1],
        cache_misses: values[2],
        instructions: values[3],
        cycles: values[4],
    })
}

pub fn close() {
    if let Some(group) = COUNTERS.lock().unwrap().take() {
        for fds in &group.fds {
            for &fd in fds {
                unsafe { libc::close(fd); }
            }
        }
    }
}

pub fn compute_delta(before: &PerfSnapshot, after: &PerfSnapshot) -> PerfMetrics {
    let l1d = after.l1d_load_misses.wrapping_sub(before.l1d_load_misses);
    let llc = after.llc_load_misses.wrapping_sub(before.llc_load_misses);
    let cache = after.cache_misses.wrapping_sub(before.cache_misses);
    let insn = after.instructions.wrapping_sub(before.instructions);
    let cyc = after.cycles.wrapping_sub(before.cycles);

    let ipc = if cyc > 0 { insn as f64 / cyc as f64 } else { 0.0 };
    let kinsn = insn as f64 / 1000.0;
    let l1d_rate = if kinsn > 0.0 { l1d as f64 / kinsn } else { 0.0 };
    let llc_rate = if kinsn > 0.0 { llc as f64 / kinsn } else { 0.0 };

    PerfMetrics {
        l1d_load_misses: l1d,
        llc_load_misses: llc,
        cache_misses: cache,
        instructions: insn,
        cycles: cyc,
        ipc,
        l1d_miss_per_kinsn: l1d_rate,
        llc_miss_per_kinsn: llc_rate,
    }
}

pub fn csv_header() -> &'static str {
    "l1d_load_misses,llc_load_misses,cache_misses,instructions,cycles,ipc,l1d_miss_per_kinsn,llc_miss_per_kinsn"
}

impl PerfMetrics {
    pub fn to_csv_row(&self) -> String {
        format!("{},{},{},{},{},{:.4},{:.4},{:.4}",
                self.l1d_load_misses, self.llc_load_misses, self.cache_misses,
                self.instructions, self.cycles, self.ipc,
                self.l1d_miss_per_kinsn, self.llc_miss_per_kinsn)
    }
}

pub fn median_perf_metrics(samples: &[PerfMetrics]) -> PerfMetrics {
    if samples.is_empty() {
        return empty_metrics();
    }
    if samples.len() == 1 {
        return samples[0].clone();
    }

    fn med_u64(vals: &mut Vec<u64>) -> u64 {
        vals.sort();
        vals[vals.len() / 2]
    }
    fn med_f64(vals: &mut Vec<f64>) -> f64 {
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        vals[vals.len() / 2]
    }

    let mut l1d: Vec<u64> = samples.iter().map(|s| s.l1d_load_misses).collect();
    let mut llc: Vec<u64> = samples.iter().map(|s| s.llc_load_misses).collect();
    let mut cache: Vec<u64> = samples.iter().map(|s| s.cache_misses).collect();
    let mut insn: Vec<u64> = samples.iter().map(|s| s.instructions).collect();
    let mut cyc: Vec<u64> = samples.iter().map(|s| s.cycles).collect();
    let mut ipc: Vec<f64> = samples.iter().map(|s| s.ipc).collect();
    let mut l1d_rate: Vec<f64> = samples.iter().map(|s| s.l1d_miss_per_kinsn).collect();
    let mut llc_rate: Vec<f64> = samples.iter().map(|s| s.llc_miss_per_kinsn).collect();

    PerfMetrics {
        l1d_load_misses: med_u64(&mut l1d),
        llc_load_misses: med_u64(&mut llc),
        cache_misses: med_u64(&mut cache),
        instructions: med_u64(&mut insn),
        cycles: med_u64(&mut cyc),
        ipc: med_f64(&mut ipc),
        l1d_miss_per_kinsn: med_f64(&mut l1d_rate),
        llc_miss_per_kinsn: med_f64(&mut llc_rate),
    }
}

pub fn empty_metrics() -> PerfMetrics {
    PerfMetrics {
        l1d_load_misses: 0,
        llc_load_misses: 0,
        cache_misses: 0,
        instructions: 0,
        cycles: 0,
        ipc: 0.0,
        l1d_miss_per_kinsn: 0.0,
        llc_miss_per_kinsn: 0.0,
    }
}

fn read_cpuset_cpu_list() -> Vec<usize> {
    let content = std::fs::read_to_string("/sys/fs/cgroup/cpuset.cpus.effective")
        .or_else(|_| std::fs::read_to_string("/sys/fs/cgroup/cpuset.cpus"))
        .unwrap_or_default();
    parse_cpu_list(content.trim())
}

fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut cpus = Vec::new();
    if s.is_empty() {
        return cpus;
    }
    for part in s.split(',') {
        if let Some((lo, hi)) = part.split_once('-') {
            if let (Ok(lo), Ok(hi)) = (lo.parse::<usize>(), hi.parse::<usize>()) {
                for cpu in lo..=hi {
                    cpus.push(cpu);
                }
            }
        } else if let Ok(cpu) = part.parse::<usize>() {
            cpus.push(cpu);
        }
    }
    cpus
}
