use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::ffi::CString;

use crate::constants::*;

/// Tuning parameters that affect contention behavior
#[derive(Debug, Clone, Copy)]
pub struct TuningParams {
    pub buffer_kb: usize,     // CPU work buffer size in KB (default: 100)
    pub io_buffer_kb: usize,  // IO read/write buffer size in KB (default: 4)
    pub max_workers: usize,   // maximum thread/process count
    pub duration_secs: u64,   // measurement duration per data point (default: 5)
    pub samples: usize,       // number of samples per data point, median is taken (default: 5)
    pub step: usize,          // worker count increment per data point (default: 1)
    pub intensity: f64,       // probability of working vs sleeping per iteration (default: 1.0)
    pub chain: bool,          // auto-chain saturation → intensity sweep (default: false)
    pub warmup_secs: u64,     // warmup duration before measurement in seconds (default: 1)
}

impl Default for TuningParams {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4);
        Self {
            buffer_kb: 100,
            io_buffer_kb: 4,
            max_workers: parallelism * 4,
            duration_secs: 5,
            samples: 5,
            step: 1,
            intensity: 1.0,
            chain: false,
            warmup_secs: 1,
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

/// Run multi-pass calibration to find CPU and IO iteration counts that produce matched timings.
pub fn calibrate_operations_full(params: &TuningParams) -> CalibrationResult {
    println!("  Running calibration ({} passes, targeting <5% gap)...", CALIBRATION_PASSES);

    let buffer_size = params.buffer_kb * 1024;
    let io_buffer_size = params.io_buffer_kb * 1024;
    let mut results = Vec::new();

    for pass in 0..CALIBRATION_PASSES {
        let (cpu_iters, cpu_us, io_iters, io_us) = calibrate_single_pass(buffer_size, io_buffer_size);
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

fn calibrate_single_pass(buffer_size: usize, io_buffer_size: usize) -> (usize, u128, usize, u128) {

    // Measure a single IO operation (O_SYNC write) directly
    let (io_iters, io_actual_us) = {
        let path = "/tmp/saturator_calibrate";
        let _ = std::fs::write(path, vec![0u8; io_buffer_size]);
        let io_buf = vec![0u8; io_buffer_size];

        let samples = CALIBRATION_IO_SAMPLES;
        let start = std::time::Instant::now();
        for _ in 0..samples {
            do_io_work_counted(path, 1, &io_buf);
        }
        let actual_us = (start.elapsed().as_micros() / samples as u128).max(1);
        let _ = std::fs::remove_file(path);
        (1usize, actual_us)
    };

    // Calibrate CPU to match I/O timing as closely as possible
    let (cpu_iters, cpu_actual_us) = {
        let buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
        let target_cpu_us = io_actual_us;
        let mut iters = 10;

        // First, get close with scaling
        loop {
            let start = std::time::Instant::now();
            let samples = CALIBRATION_CPU_SCALING_SAMPLES;
            for _ in 0..samples {
                do_cpu_work(&buffer, iters);
            }
            let actual_us = start.elapsed().as_micros() / samples as u128;

            if actual_us >= target_cpu_us {
                break;
            }

            let scale = (target_cpu_us / actual_us.max(1)).max(2) as usize;
            iters *= scale;

            if iters > CALIBRATION_MAX_CPU_ITERS {
                break;
            }
        }

        // Fine-tune with binary search to get within tolerance
        let mut low = iters / 2;
        let mut high = iters * 2;
        let lower_bound = target_cpu_us * (100 - CALIBRATION_TOLERANCE_PCT) / 100;
        let upper_bound = target_cpu_us * (100 + CALIBRATION_TOLERANCE_PCT) / 100;

        for _ in 0..CALIBRATION_MAX_SEARCH_ITERS {
            let mid = (low + high) / 2;
            if mid == low || mid == high {
                break;
            }

            let start = std::time::Instant::now();
            let samples = CALIBRATION_CPU_SEARCH_SAMPLES;
            for _ in 0..samples {
                do_cpu_work(&buffer, mid);
            }
            let actual_us = start.elapsed().as_micros() / samples as u128;

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
        let samples = CALIBRATION_CPU_FINAL_SAMPLES;
        for _ in 0..samples {
            do_cpu_work(&buffer, iters);
        }
        let actual_us = start.elapsed().as_micros() / samples as u128;

        (iters, actual_us)
    };

    (cpu_iters, cpu_actual_us, io_iters, io_actual_us)
}

/// Thread work loop: runs CPU and IO work at the given ratio until `running` is set to false.
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
    pt_cpu: Arc<AtomicU64>,
    pt_io: Arc<AtomicU64>,
    pt_sleep: Arc<AtomicU64>,
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
    let mut local_sleep_ops = 0u64;
    let mut rng_state = thread_id as u64 + RNG_SEED_OFFSET;

    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                local_sleep_ops += 1;
                if local_sleep_ops >= BATCH_FLUSH_THRESHOLD {
                    pt_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
                    local_sleep_ops = 0;
                }
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

        if (local_cpu_ops + local_io_ops) >= BATCH_FLUSH_THRESHOLD {
            cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            pt_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
            pt_io.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    if local_cpu_ops > 0 {
        cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
        pt_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
        pt_io.fetch_add(local_io_ops, Ordering::Relaxed);
    }
    if local_sleep_ops > 0 {
        pt_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
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
        let offset = (i * CPU_BUFFER_STRIDE) % len;
        let end = (offset + CPU_BUFFER_STRIDE).min(len);
        hash = buffer[offset..end].iter().fold(hash, |acc, &b| {
            acc.wrapping_mul(HASH_MUL_PRIMARY).wrapping_add(b as u64)
        });

        for j in 0..CPU_HASH_UNROLL {
            hash = hash.wrapping_add(j).wrapping_mul(HASH_MUL_SECONDARY);
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

/// Public wrapper around `do_cpu_work` for use outside the workload loop.
pub fn cpu_work_with_buffer(buffer: &[u8], iterations: usize) {
    do_cpu_work(buffer, iterations);
}

/// Public wrapper around `do_io_work_counted` that manages a per-thread scratch file.
pub fn io_work_with_id(thread_id: usize, iterations: usize, io_buf: &[u8]) {
    let path = format!("/tmp/saturator_io_{}", thread_id);

    // Create file once if it doesn't exist
    if !std::path::Path::new(&path).exists() {
        let _ = std::fs::write(&path, vec![0u8; io_buf.len()]);
    }

    do_io_work_counted(&path, iterations, io_buf);
}

// --- Shared memory infrastructure for process-based experiments ---

/// Cross-process shared memory region with atomic counters for throughput tracking.
#[repr(C)]
pub struct SharedRegion {
    pub total_ops: AtomicU64,
    pub cpu_ops: AtomicU64,
    pub io_ops: AtomicU64,
    pub ready_count: AtomicU64,  // children increment when setup is complete
    pub running: AtomicBool,
    _padding: [u8; 7],
}

/// Compute shared memory size: header + per-worker counters, rounded up to page size.
fn shm_size_for_workers(max_workers: usize) -> usize {
    let header = std::mem::size_of::<SharedRegion>();
    let per_worker = max_workers * SHM_BYTES_PER_WORKER;
    let total = header + per_worker;
    (total + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Create a named POSIX shared memory region for `max_workers` workers. Returns (pointer, fd).
pub fn create_shared_region(name: &str, max_workers: usize) -> (*mut SharedRegion, i32) {
    let size = shm_size_for_workers(max_workers);
    unsafe {
        let c_name = CString::new(name).unwrap();
        let fd = libc::shm_open(
            c_name.as_ptr(),
            libc::O_CREAT | libc::O_RDWR,
            0o600,
        );
        assert!(fd >= 0, "shm_open failed: {}", std::io::Error::last_os_error());

        assert_eq!(libc::ftruncate(fd, size as libc::off_t), 0,
            "ftruncate failed: {}", std::io::Error::last_os_error());

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
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

        // Zero-init per-worker area
        let worker_area = (ptr as *mut u8).add(std::mem::size_of::<SharedRegion>());
        std::ptr::write_bytes(worker_area, 0, max_workers * SHM_BYTES_PER_WORKER);

        (region, fd)
    }
}

/// Open an existing named shared memory region (used by child processes).
pub fn open_shared_region(name: &str, max_workers: usize) -> *mut SharedRegion {
    let size = shm_size_for_workers(max_workers);
    unsafe {
        let c_name = CString::new(name).unwrap();
        let fd = libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0);
        assert!(fd >= 0, "shm_open (child) failed: {}", std::io::Error::last_os_error());

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size,
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

/// Unmap and unlink a shared memory region, closing the file descriptor.
pub fn destroy_shared_region(name: &str, ptr: *mut SharedRegion, fd: i32, max_workers: usize) {
    let size = shm_size_for_workers(max_workers);
    unsafe {
        libc::munmap(ptr as *mut libc::c_void, size);
        libc::close(fd);
        let c_name = CString::new(name).unwrap();
        libc::shm_unlink(c_name.as_ptr());
    }
}

/// Get per-worker atomic counters (cpu_ops, io_ops, sleep_ops) from the shared region.
/// Worker counters are laid out after the SharedRegion header: [AtomicU64 cpu, AtomicU64 io, AtomicU64 sleep] per worker.
///
/// # Safety
/// Caller must ensure `region` points to a valid shared region with space for `worker_id` workers.
pub unsafe fn worker_counters(region: *mut SharedRegion, worker_id: usize) -> (&'static AtomicU64, &'static AtomicU64, &'static AtomicU64) {
    unsafe {
        let base = (region as *mut u8).add(std::mem::size_of::<SharedRegion>());
        let slot = base.add(worker_id * SHM_BYTES_PER_WORKER);
        let cpu   = &*(slot as *const AtomicU64);
        let io    = &*((slot.add(8))  as *const AtomicU64);
        let sleep = &*((slot.add(16)) as *const AtomicU64);
        (cpu, io, sleep)
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
    max_workers: usize,
) {
    let region = open_shared_region(shm_name, max_workers);
    let region_ref = unsafe { &*region };
    let (wk_cpu, wk_io, wk_sleep) = unsafe { worker_counters(region, worker_id) };

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
    let mut local_sleep_ops = 0u64;
    let mut rng_state = worker_id as u64 + RNG_SEED_OFFSET;

    while region_ref.running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;

        // Intensity gate: sleep instead of working with probability (1 - intensity)
        if intensity < 1.0 {
            rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
            let intensity_roll = (rng_state >> 32) as f64 / u32::MAX as f64;
            if intensity_roll >= intensity {
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
                local_sleep_ops += 1;
                if local_sleep_ops >= BATCH_FLUSH_THRESHOLD {
                    wk_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
                    local_sleep_ops = 0;
                }
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

        if (local_cpu_ops + local_io_ops) >= BATCH_FLUSH_THRESHOLD {
            region_ref.cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
            region_ref.io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
            wk_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
            wk_io.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    if local_cpu_ops > 0 {
        region_ref.cpu_ops.fetch_add(local_cpu_ops, Ordering::Relaxed);
        wk_cpu.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        region_ref.io_ops.fetch_add(local_io_ops, Ordering::Relaxed);
        wk_io.fetch_add(local_io_ops, Ordering::Relaxed);
    }
    if local_sleep_ops > 0 {
        wk_sleep.fetch_add(local_sleep_ops, Ordering::Relaxed);
    }

    let _ = std::fs::remove_file(&io_path);
}
