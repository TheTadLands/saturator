use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::ffi::CString;

/// Tuning parameters that affect contention behavior
#[derive(Debug, Clone, Copy)]
pub struct TuningParams {
    pub target_us: u128,      // calibration target per-operation (default: 50)
    pub buffer_kb: usize,     // CPU work buffer size in KB (default: 100)
    pub io_buffer_kb: usize,  // IO read/write buffer size in KB (default: 4)
    pub max_workers: usize,   // maximum thread/process count
    pub duration_secs: u64,   // measurement duration per data point (default: 5)
    pub samples: usize,       // number of samples per data point, median is taken (default: 3)
    pub step: usize,          // worker count increment per data point (default: 1)
    pub intensity: f64,       // probability of working vs sleeping per iteration (default: 1.0)
}

impl Default for TuningParams {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4);
        Self {
            target_us: 50,
            buffer_kb: 100,
            io_buffer_kb: 4,
            max_workers: parallelism * 4,
            duration_secs: 5,
            samples: 3,
            step: 1,
            intensity: 1.0,
        }
    }
}

/// Calibration result containing iterations and timing info
#[derive(Debug, Clone, Copy)]
pub struct CalibrationResult {
    pub cpu_iterations: usize,
    pub io_iterations: usize,
    pub cpu_us: u128,
    pub io_us: u128,
}

impl CalibrationResult {
    /// Returns ops/second for CPU work
    pub fn cpu_ops_per_sec(&self) -> f64 {
        if self.cpu_us == 0 { return 0.0; }
        1_000_000.0 / self.cpu_us as f64
    }

    /// Returns ops/second for I/O work
    pub fn io_ops_per_sec(&self) -> f64 {
        if self.io_us == 0 { return 0.0; }
        1_000_000.0 / self.io_us as f64
    }
}

pub fn calibrate_operations_full(params: &TuningParams) -> CalibrationResult {
    println!("  Running calibration (3 passes, targeting <5% gap)...");

    let buffer_size = params.buffer_kb * 1024;
    let io_buffer_size = params.io_buffer_kb * 1024;
    let mut results = Vec::new();

    for pass in 0..3 {
        let (cpu_iters, cpu_us, io_iters, io_us) = calibrate_single_pass(params.target_us, buffer_size, io_buffer_size);
        let gap_pct = if cpu_us > io_us {
            ((cpu_us - io_us) as f64 / io_us as f64) * 100.0
        } else {
            ((io_us - cpu_us) as f64 / cpu_us as f64) * 100.0
        };
        println!("    Pass {}: CPU {} iters = {}μs, I/O {} iters = {}μs (gap: {:.1}%)",
                 pass + 1, cpu_iters, cpu_us, io_iters, io_us, gap_pct);
        results.push((cpu_iters, cpu_us, io_iters, io_us));
    }

    // Take median based on CPU iterations
    results.sort_by_key(|(cpu_iters, _, _, _)| *cpu_iters);
    let (cpu_iters, cpu_us, io_iters, io_us) = results[1]; // median

    let gap_pct = if cpu_us > io_us {
        ((cpu_us - io_us) as f64 / io_us as f64) * 100.0
    } else {
        ((io_us - cpu_us) as f64 / cpu_us as f64) * 100.0
    };

    let cpu_ops_sec = 1_000_000.0 / cpu_us as f64;
    let io_ops_sec = 1_000_000.0 / io_us as f64;

    println!("  Final: CPU {} iters = {}μs ({:.0} ops/s), I/O {} iters = {}μs ({:.0} ops/s), gap: {:.1}%",
             cpu_iters, cpu_us, cpu_ops_sec, io_iters, io_us, io_ops_sec, gap_pct);

    CalibrationResult {
        cpu_iterations: cpu_iters,
        io_iterations: io_iters,
        cpu_us,
        io_us,
    }
}

fn calibrate_single_pass(target_us: u128, buffer_size: usize, io_buffer_size: usize) -> (usize, u128, usize, u128) {

    // Calibrate I/O first (coarser granularity)
    let (io_iters, io_actual_us) = {
        let path = "/tmp/saturator_calibrate";
        let _ = std::fs::write(path, vec![0u8; io_buffer_size]);
        let io_buf = vec![0u8; io_buffer_size];

        let mut iters = 1;
        let mut actual_us = 0u128;

        loop {
            let start = std::time::Instant::now();
            let samples = 100;
            for _ in 0..samples {
                do_io_work_counted(path, iters, &io_buf);
            }
            actual_us = start.elapsed().as_micros() / samples as u128;

            if actual_us >= target_us {
                let _ = std::fs::remove_file(path);
                break (iters, actual_us);
            }

            let scale = (target_us / actual_us.max(1)).max(2) as usize;
            iters = (iters * scale).max(iters + 1);

            if iters > 10000 {
                let _ = std::fs::remove_file(path);
                break (iters, actual_us);
            }
        }
    };

    // Calibrate CPU to match I/O timing as closely as possible
    let (cpu_iters, cpu_actual_us) = {
        let buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
        let target_cpu_us = io_actual_us;
        let mut iters = 10;
        let mut actual_us = 0u128;

        // First, get close with scaling
        loop {
            let start = std::time::Instant::now();
            let samples = 200; // More samples for precision
            for _ in 0..samples {
                do_cpu_work(&buffer, iters);
            }
            actual_us = start.elapsed().as_micros() / samples as u128;

            if actual_us >= target_cpu_us {
                break;
            }

            let scale = (target_cpu_us / actual_us.max(1)).max(2) as usize;
            iters *= scale;

            if iters > 1_000_000 {
                break;
            }
        }

        // Fine-tune with binary search to get within 2%
        let mut low = iters / 2;
        let mut high = iters * 2;
        let lower_bound = target_cpu_us * 98 / 100;
        let upper_bound = target_cpu_us * 102 / 100;

        for _ in 0..40 {
            let mid = (low + high) / 2;
            if mid == low || mid == high {
                break;
            }

            let start = std::time::Instant::now();
            let samples = 500;
            for _ in 0..samples {
                do_cpu_work(&buffer, mid);
            }
            actual_us = start.elapsed().as_micros() / samples as u128;

            if actual_us >= lower_bound && actual_us <= upper_bound {
                iters = mid;
                break;
            } else if actual_us < target_cpu_us {
                low = mid;
            } else {
                high = mid;
            }
            iters = mid;
        }

        // Final measurement with more samples for accuracy
        let start = std::time::Instant::now();
        let samples = 1000;
        for _ in 0..samples {
            do_cpu_work(&buffer, iters);
        }
        actual_us = start.elapsed().as_micros() / samples as u128;

        (iters, actual_us)
    };

    (cpu_iters, cpu_actual_us, io_iters, io_actual_us)
}

pub fn run_saturator(
    thread_id: usize,
    io_perc: f64,
    cpu_iterations: usize,
    io_iterations: usize,
    buffer_size: usize,
    io_buffer_size: usize,
    cpu_ops: Arc<AtomicU64>,
    io_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    intensity: f64,
    sleep_us: u64,
) {
    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_{}", thread_id);
    let io_buf = vec![0u8; io_buffer_size];

    // Create file for I/O
    if io_perc > 0.0 {
        let _ = std::fs::write(&io_path, vec![0u8; io_buffer_size]);
    }

    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;
    let mut rng_state = thread_id as u64 + 12345;

    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                continue;
            }
        }

        if rand_f64 < io_perc {
            do_io_work_counted(&io_path, io_iterations, &io_buf);
            local_io_ops += io_iterations as u64;
        } else {
            do_cpu_work(&cpu_buffer, cpu_iterations);
            local_cpu_ops += cpu_iterations as u64;
        }

        if (local_cpu_ops + local_io_ops) >= 100 {
            cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    if local_cpu_ops > 0 {
        cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
    }

    // Always try to clean up
    let _ = std::fs::remove_file(&io_path);
}

fn do_cpu_work(buffer: &[u8], iterations: usize) -> u64 {
    let mut hash = 0u64;
    let len = buffer.len();

    for i in 0..iterations {
        // Stride through the entire buffer so the full working set stays hot.
        // With large buffers and many processes, this creates real cache pressure.
        let offset = (i * 1000) % len;
        let end = (offset + 1000).min(len);
        hash = buffer[offset..end].iter().fold(hash, |acc, &b| {
            acc.wrapping_mul(31).wrapping_add(b as u64)
        });

        for j in 0..50 {
            hash = hash.wrapping_add(j).wrapping_mul(7);
        }
    }

    std::hint::black_box(hash)
}

fn do_io_work_counted(path: &str, iterations: usize, io_buf: &[u8]) {
    for _ in 0..iterations {
        // O_SYNC write — blocks until data hits disk
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_SYNC)
            .open(path)
        {
            let _ = file.write_all(io_buf);
        }
    }
}

pub fn cpu_work_with_buffer(buffer: &[u8], iterations: usize) {
    do_cpu_work(buffer, iterations);
}

pub fn io_work_with_id(thread_id: usize, iterations: usize, io_buf: &[u8]) {
    let path = format!("/tmp/saturator_io_{}", thread_id);

    // Create file once if it doesn't exist
    if !std::path::Path::new(&path).exists() {
        let _ = std::fs::write(&path, vec![0u8; io_buf.len()]);
    }

    do_io_work_counted(&path, iterations, io_buf);
}

// --- Shared memory infrastructure for process-based experiments ---

#[repr(C)]
pub struct SharedRegion {
    pub total_ops: AtomicU64,
    pub cpu_ops: AtomicU64,
    pub io_ops: AtomicU64,
    pub ready_count: AtomicU64,  // children increment when setup is complete
    pub running: AtomicBool,
    _padding: [u8; 7],
}

const SHM_SIZE: usize = 4096; // one page, plenty for SharedRegion

pub fn create_shared_region(name: &str) -> (*mut SharedRegion, i32) {
    unsafe {
        let c_name = CString::new(name).unwrap();
        let fd = libc::shm_open(
            c_name.as_ptr(),
            libc::O_CREAT | libc::O_RDWR,
            0o600,
        );
        assert!(fd >= 0, "shm_open failed: {}", std::io::Error::last_os_error());

        assert_eq!(libc::ftruncate(fd, SHM_SIZE as libc::off_t), 0,
            "ftruncate failed: {}", std::io::Error::last_os_error());

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            SHM_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        assert!(ptr != libc::MAP_FAILED, "mmap failed: {}", std::io::Error::last_os_error());

        let region = ptr as *mut SharedRegion;
        // Initialize all fields
        std::ptr::write(&mut (*region).total_ops, AtomicU64::new(0));
        std::ptr::write(&mut (*region).cpu_ops, AtomicU64::new(0));
        std::ptr::write(&mut (*region).io_ops, AtomicU64::new(0));
        std::ptr::write(&mut (*region).ready_count, AtomicU64::new(0));
        std::ptr::write(&mut (*region).running, AtomicBool::new(true));

        (region, fd)
    }
}

pub fn open_shared_region(name: &str) -> *mut SharedRegion {
    unsafe {
        let c_name = CString::new(name).unwrap();
        let fd = libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0);
        assert!(fd >= 0, "shm_open (child) failed: {}", std::io::Error::last_os_error());

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            SHM_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        assert!(ptr != libc::MAP_FAILED, "mmap (child) failed");

        libc::close(fd);
        ptr as *mut SharedRegion
    }
}

pub fn destroy_shared_region(name: &str, ptr: *mut SharedRegion, fd: i32) {
    unsafe {
        libc::munmap(ptr as *mut libc::c_void, SHM_SIZE);
        libc::close(fd);
        let c_name = CString::new(name).unwrap();
        libc::shm_unlink(c_name.as_ptr());
    }
}

/// Child process entry point — opens shared memory and runs workload loop
pub fn run_worker_process(
    shm_name: &str,
    worker_id: usize,
    cpu_iterations: usize,
    io_iterations: usize,
    io_perc: f64,
    buffer_size: usize,
    io_buffer_size: usize,
    intensity: f64,
    sleep_us: u64,
) {
    let region = open_shared_region(shm_name);
    let region_ref = unsafe { &*region };

    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_proc_{}", worker_id);
    let io_buf = vec![0u8; io_buffer_size];

    if io_perc > 0.0 {
        let _ = std::fs::write(&io_path, vec![0u8; io_buffer_size]);
    }

    // Signal that setup is complete
    region_ref.ready_count.fetch_add(1, Ordering::Relaxed);

    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;
    let mut rng_state = worker_id as u64 + 12345;

    while region_ref.running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                continue;
            }
        }

        if rand_f64 < io_perc {
            do_io_work_counted(&io_path, io_iterations, &io_buf);
            local_io_ops += io_iterations as u64;
        } else {
            do_cpu_work(&cpu_buffer, cpu_iterations);
            local_cpu_ops += cpu_iterations as u64;
        }

        if (local_cpu_ops + local_io_ops) >= 100 {
            region_ref.cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            region_ref.io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    if local_cpu_ops > 0 {
        region_ref.cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        region_ref.io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
    }

    let _ = std::fs::remove_file(&io_path);
}
