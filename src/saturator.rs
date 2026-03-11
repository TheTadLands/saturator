use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::ffi::CString;

use crate::constants::*;

/// Read CLOCK_MONOTONIC and return nanoseconds. This is a vDSO call on Linux (~20ns).
pub fn now_ns() -> u64 {
    unsafe {
        let mut ts = std::mem::MaybeUninit::<libc::timespec>::uninit();
        libc::clock_gettime(libc::CLOCK_MONOTONIC, ts.as_mut_ptr());
        let ts = ts.assume_init();
        ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
    }
}

/// Tuning parameters that affect contention behavior
#[derive(Debug, Clone, Copy)]
pub struct TuningParams {
    pub buffer_kb: usize,     // CPU work buffer size in KB (default: 100)
    pub io_buffer_kb: usize,  // IO read/write buffer size in KB (default: 4)
    pub max_workers: usize,   // maximum thread/process count
    pub duration_secs: u64,   // measurement duration per data point (default: 30)
    pub samples: usize,       // number of samples per data point, median is taken (default: 5)
    pub step: usize,          // worker count increment per data point (default: 1)
    pub intensity: f64,       // probability of working vs sleeping per iteration (default: 1.0)
    pub chain: bool,          // auto-chain saturation → intensity sweep (default: false)
    pub warmup_secs: u64,     // warmup duration before measurement in seconds (default: 10)
    pub random_access: bool,  // use random buffer access pattern to defeat prefetcher (default: false)
    pub direct_io: bool,      // use O_DIRECT to bypass page cache for I/O ops (default: false)
}

impl Default for TuningParams {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(|n| n.get()).unwrap_or(4);
        Self {
            buffer_kb: 100,
            io_buffer_kb: 4,
            max_workers: parallelism * 4,
            duration_secs: 30,
            samples: 5,
            step: 1,
            intensity: 1.0,
            chain: false,
            warmup_secs: 10,
            random_access: false,
            direct_io: false,
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
        let (cpu_iters, cpu_us, io_iters, io_us) = calibrate_single_pass(buffer_size, io_buffer_size, params.random_access, params.direct_io);
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
    let (cpu_iters, cpu_us, io_iters, io_us) = results[results.len() / 2];

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

fn calibrate_single_pass(buffer_size: usize, io_buffer_size: usize, random_access: bool, direct_io: bool) -> (usize, u128, usize, u128) {

    // Measure a single IO operation (O_SYNC write) directly
    let (io_iters, io_actual_us) = {
        let path = "/tmp/saturator_calibrate";
        let io_buf: &[u8] = if direct_io {
            alloc_aligned_io_buf(io_buffer_size)
        } else {
            // Leak a regular vec to get a &'static [u8] — calibration runs once
            Vec::leak(vec![0u8; io_buffer_size])
        };
        let mut io = IoState::new(path, io_buffer_size, direct_io);

        let samples = CALIBRATION_IO_SAMPLES;
        let start = std::time::Instant::now();
        for _ in 0..samples {
            do_io_work(&mut io, 1, io_buf);
        }
        let actual_us = (start.elapsed().as_micros() / samples as u128).max(1);
        drop(io);
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
                do_cpu_work(&buffer, iters, random_access);
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
                do_cpu_work(&buffer, mid, random_access);
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
            do_cpu_work(&buffer, iters, random_access);
        }
        let actual_us = start.elapsed().as_micros() / samples as u128;

        (iters, actual_us)
    };

    (cpu_iters, cpu_actual_us, io_iters, io_actual_us)
}

/// Core work loop shared by thread-based and process-based workers.
/// Runs CPU/IO work at the given ratio, flushing counters periodically,
/// until the deadline (read from `deadline_ns`) is reached.
fn work_loop(
    deadline_ns: &AtomicU64,
    cpu_counter: &AtomicU64,
    io_counter: &AtomicU64,
    sleep_counter: &AtomicU64,
    cpu_buffer: &[u8],
    io_state: &mut Option<IoState>,
    io_buf: &[u8],
    cpu_iterations: usize,
    io_iterations: usize,
    io_perc: f64,
    intensity: f64,
    sleep_us: u64,
    random_access: bool,
    rng_seed: u64,
) {
    let mut local_cpu_ops = 0u64;
    let mut local_io_ops = 0u64;
    let mut local_sleep_ops = 0u64;
    let mut rng_state = rng_seed;

    loop {
        let d = deadline_ns.load(Ordering::Relaxed);
        if d != 0 && now_ns() >= d {
            break;
        }

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
                    sleep_counter.fetch_add(local_sleep_ops, Ordering::Relaxed);
                    local_sleep_ops = 0;
                }
                continue;
            }
        }

        if rand_f64 < io_perc {
            if let Some(io) = io_state {
                do_io_work(io, io_iterations, io_buf);
            }
            local_io_ops += io_iterations as u64;
        } else {
            do_cpu_work(cpu_buffer, cpu_iterations, random_access);
            local_cpu_ops += cpu_iterations as u64;
        }

        if (local_cpu_ops + local_io_ops) >= BATCH_FLUSH_THRESHOLD {
            cpu_counter.fetch_add(local_cpu_ops, Ordering::Relaxed);
            io_counter.fetch_add(local_io_ops, Ordering::Relaxed);
            local_cpu_ops = 0;
            local_io_ops = 0;
        }
    }

    if local_cpu_ops > 0 {
        cpu_counter.fetch_add(local_cpu_ops, Ordering::Relaxed);
    }
    if local_io_ops > 0 {
        io_counter.fetch_add(local_io_ops, Ordering::Relaxed);
    }
    if local_sleep_ops > 0 {
        sleep_counter.fetch_add(local_sleep_ops, Ordering::Relaxed);
    }
}

/// Thread work loop: sets up buffers and IO state, runs the shared work loop,
/// then records errors and cleans up.
pub fn run_saturator(
    thread_id: usize,
    io_perc: f64,
    deadline_ns: Arc<AtomicU64>,
    pt_cpu: Arc<AtomicU64>,
    pt_io: Arc<AtomicU64>,
    pt_sleep: Arc<AtomicU64>,
    pt_errors: Arc<AtomicU64>,
    params: TuningParams,
    calibration: CalibrationResult,
) {
    let buffer_size = params.buffer_kb * 1024;
    let io_buffer_size = params.io_buffer_kb * 1024;

    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_{}", thread_id);
    let aligned_buf = if params.direct_io { Some(AlignedBuf::new(io_buffer_size)) } else { None };
    let io_vec = if params.direct_io { None } else { Some(vec![0u8; io_buffer_size]) };
    let io_buf: &[u8] = if let Some(ref ab) = aligned_buf { ab.as_slice() } else { io_vec.as_ref().unwrap() };

    let mut io_state = if io_perc > 0.0 {
        Some(IoState::new(&io_path, io_buffer_size, params.direct_io))
    } else {
        None
    };

    work_loop(
        &deadline_ns, &pt_cpu, &pt_io, &pt_sleep,
        &cpu_buffer, &mut io_state, io_buf,
        calibration.cpu_iterations, calibration.io_iterations,
        io_perc, params.intensity, calibration.cpu_us as u64,
        params.random_access, thread_id as u64 + RNG_SEED_OFFSET,
    );

    if let Some(ref io) = io_state {
        if io.errors > 0 {
            pt_errors.fetch_add(io.errors, Ordering::Relaxed);
        }
    }
    drop(io_state);
    let _ = std::fs::remove_file(&io_path);
}

fn do_cpu_work(buffer: &[u8], iterations: usize, random_access: bool) -> u64 {
    let mut hash = 0u64;
    let len = buffer.len();

    for i in 0..iterations {
        let offset = if random_access {
            // Use running hash to derive offset — pointer-chasing pattern defeats prefetcher
            (hash as usize) % len.saturating_sub(CPU_BUFFER_STRIDE).max(1)
        } else {
            // Sequential stride — prefetcher-friendly
            (i * CPU_BUFFER_STRIDE) % len
        };
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

/// RAII wrapper around a page-aligned buffer allocated via `posix_memalign`.
pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

impl AlignedBuf {
    pub fn new(size: usize) -> Self {
        unsafe {
            let mut ptr: *mut libc::c_void = std::ptr::null_mut();
            let ret = libc::posix_memalign(&mut ptr, 4096, size);
            assert_eq!(ret, 0, "posix_memalign failed");
            std::ptr::write_bytes(ptr as *mut u8, 0, size);
            Self { ptr: ptr as *mut u8, len: size }
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { libc::free(self.ptr as *mut libc::c_void); }
    }
}

// Safety: AlignedBuf owns its allocation and is not shared.
unsafe impl Send for AlignedBuf {}

/// Allocate a page-aligned zero buffer (required for O_DIRECT). Never freed.
/// Use only where leaking is acceptable (calibration, child processes that exit immediately).
pub fn alloc_aligned_io_buf(size: usize) -> &'static [u8] {
    unsafe {
        let mut ptr: *mut libc::c_void = std::ptr::null_mut();
        let ret = libc::posix_memalign(&mut ptr, 4096, size);
        assert_eq!(ret, 0, "posix_memalign failed");
        std::ptr::write_bytes(ptr as *mut u8, 0, size);
        std::slice::from_raw_parts(ptr as *const u8, size)
    }
}

/// RAII wrapper for an IO scratch file. Opens once, preallocates, writes at advancing offsets.
pub struct IoState {
    fd: i32,
    offset: usize,
    file_size: usize,
    block_bytes: usize,
    pub errors: u64,
}

impl IoState {
    pub fn new(path: &str, block_bytes: usize, direct_io: bool) -> Self {
        if direct_io && block_bytes % 4096 != 0 {
            panic!(
                "O_DIRECT requires io-buffer-kb to be a multiple of 4KB, got {} bytes ({} KB). \
                 Use --io-buffer-kb with a multiple of 4 (e.g. 4, 8, 16).",
                block_bytes, block_bytes / 1024
            );
        }

        let file_size = IO_FILE_SIZE_BYTES.max(256 * block_bytes);
        let mut flags = libc::O_CREAT | libc::O_RDWR | libc::O_SYNC;
        if direct_io {
            flags |= libc::O_DIRECT;
        }

        let c_path = CString::new(path).unwrap();
        let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o600) };
        assert!(fd >= 0, "IoState: open({}) failed: {}", path, std::io::Error::last_os_error());

        let ret = unsafe { libc::posix_fallocate(fd, 0, file_size as libc::off_t) };
        assert_eq!(ret, 0, "IoState: posix_fallocate failed: {}", std::io::Error::last_os_error());

        Self { fd, offset: 0, file_size, block_bytes, errors: 0 }
    }

    pub fn write_once(&mut self, buf: &[u8]) {
        let written = unsafe {
            libc::pwrite(self.fd, buf.as_ptr() as *const libc::c_void, self.block_bytes, self.offset as libc::off_t)
        };
        if written < 0 {
            self.errors += 1;
        }
        self.offset = (self.offset + self.block_bytes) % self.file_size;
    }
}

impl Drop for IoState {
    fn drop(&mut self) {
        if self.errors > 0 {
            eprintln!("warning: IoState encountered {} IO errors", self.errors);
        }
        unsafe { libc::close(self.fd); }
    }
}

fn do_io_work(io: &mut IoState, iterations: usize, io_buf: &[u8]) {
    for _ in 0..iterations {
        io.write_once(io_buf);
    }
}

// --- Shared memory infrastructure for process-based experiments ---

/// Cross-process shared memory region with atomic counters for throughput tracking.
#[repr(C)]
pub struct SharedRegion {
    pub cpu_ops: AtomicU64,
    pub io_ops: AtomicU64,
    pub ready_count: AtomicU64,  // children increment when setup is complete
    pub deadline_ns: AtomicU64,  // 0 = no deadline (warmup), >0 = stop when clock >= this
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
        std::ptr::write(&mut (*region).cpu_ops, AtomicU64::new(0));
        std::ptr::write(&mut (*region).io_ops, AtomicU64::new(0));
        std::ptr::write(&mut (*region).ready_count, AtomicU64::new(0));
        std::ptr::write(&mut (*region).deadline_ns, AtomicU64::new(0));

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

/// Get per-worker atomic counters (cpu_ops, io_ops, sleep_ops, io_errors) from the shared region.
/// Worker counters are laid out after the SharedRegion header: [AtomicU64 cpu, AtomicU64 io, AtomicU64 sleep, AtomicU64 errors] per worker.
///
/// # Safety
/// Caller must ensure `region` points to a valid shared region with space for `worker_id` workers.
pub unsafe fn worker_counters(region: *mut SharedRegion, worker_id: usize) -> (&'static AtomicU64, &'static AtomicU64, &'static AtomicU64, &'static AtomicU64) {
    unsafe {
        let base = (region as *mut u8).add(std::mem::size_of::<SharedRegion>());
        let slot = base.add(worker_id * SHM_BYTES_PER_WORKER);
        let cpu    = &*(slot as *const AtomicU64);
        let io     = &*((slot.add(8))  as *const AtomicU64);
        let sleep  = &*((slot.add(16)) as *const AtomicU64);
        let errors = &*((slot.add(24)) as *const AtomicU64);
        (cpu, io, sleep, errors)
    }
}

/// Child process entry point — opens shared memory, runs the shared work loop,
/// then records errors and cleans up.
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
    random_access: bool,
    direct_io: bool,
) {
    let region = open_shared_region(shm_name, max_workers);
    let region_ref = unsafe { &*region };
    let (wk_cpu, wk_io, wk_sleep, wk_errors) = unsafe { worker_counters(region, worker_id) };

    let cpu_buffer: Vec<u8> = (0..buffer_size).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_proc_{}", worker_id);
    let io_buf: &[u8] = if direct_io {
        alloc_aligned_io_buf(io_buffer_size)
    } else {
        Vec::leak(vec![0u8; io_buffer_size])
    };

    let mut io_state = if io_perc > 0.0 {
        Some(IoState::new(&io_path, io_buffer_size, direct_io))
    } else {
        None
    };

    // Signal that setup is complete
    region_ref.ready_count.fetch_add(1, Ordering::Relaxed);

    work_loop(
        &region_ref.deadline_ns, wk_cpu, wk_io, wk_sleep,
        &cpu_buffer, &mut io_state, io_buf,
        cpu_iterations, io_iterations,
        io_perc, intensity, sleep_us,
        random_access, worker_id as u64 + RNG_SEED_OFFSET,
    );

    if let Some(ref io) = io_state {
        if io.errors > 0 {
            wk_errors.fetch_add(io.errors, Ordering::Relaxed);
        }
    }
    drop(io_state);
    let _ = std::fs::remove_file(&io_path);
}
