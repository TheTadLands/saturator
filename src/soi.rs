use std::sync::atomic::{AtomicU64, Ordering};
use std::ffi::CString;
use std::fmt;

use crate::constants::*;
use crate::saturator::{open_shared_region, worker_counters, now_ns};

/// Available Sources of Interference, each targeting a specific shared resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoiType {
    L1d,
    L2,
    L3,
    MemBw,
    MemCap,
    Cpu,
    IoBw,
    IoOps,
}

/// How a SoI applies pressure: an in-process synthetic worker, or an external
/// command spawned as a child. External backends bypass the shmem ready/deadline
/// rendezvous and the per-worker counter shape — they're started, run for the
/// full warmup+measurement window, and SIGTERM-ed at teardown.
pub enum SoiBackend {
    Internal,
    /// Shell command template. Supported placeholders: `{n}` (worker count),
    /// `{runtime}` (wall seconds the SoI must stay alive),
    /// `{scratch_dir}` (per-run scratch directory, created by caller).
    External(&'static str),
}

/// fio profile for sustained sequential write bandwidth pressure. `numjobs={n}`
/// scales pressure via fio's native multi-job model rather than spawning N fio
/// processes (which would each carry their own io_uring/libaio state). Direct IO
/// bypasses the page cache so the bytes actually hit the device queue.
const IOBW_FIO_TEMPLATE: &str = "exec fio \
    --name=saturator_iobw \
    --rw=write --bs=128k --direct=1 \
    --ioengine=libaio --iodepth=32 \
    --numjobs={n} --group_reporting \
    --time_based --runtime={runtime} --size=1G \
    --directory={scratch_dir} --output=/dev/null";

/// fio profile for sustained IOPS pressure. Random offsets prevent the IO
/// scheduler and device from merging contiguous writes into bandwidth-friendly
/// batches (which would inflate bytes/sec without inflating ops/sec). 4k blocks
/// maximize ops per byte transferred against a fixed bandwidth ceiling.
const IOOPS_FIO_TEMPLATE: &str = "exec fio \
    --name=saturator_iops \
    --rw=randwrite --bs=4k --direct=1 \
    --ioengine=libaio --iodepth=32 \
    --numjobs={n} --group_reporting \
    --time_based --runtime={runtime} --size=1G \
    --directory={scratch_dir} --output=/dev/null";

impl SoiType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "l1d" | "l1" => Some(Self::L1d),
            "l2" => Some(Self::L2),
            "l3" | "llc" => Some(Self::L3),
            "membw" | "mem-bw" => Some(Self::MemBw),
            "memcap" | "mem-cap" => Some(Self::MemCap),
            "cpu" => Some(Self::Cpu),
            "iobw" | "io-bw" => Some(Self::IoBw),
            "iops" | "ioops" | "io-ops" => Some(Self::IoOps),
            _ => None,
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::L1d, Self::L2, Self::L3,
            Self::MemBw, Self::MemCap,
            Self::Cpu,
            Self::IoBw, Self::IoOps,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::L1d => "l1d",
            Self::L2 => "l2",
            Self::L3 => "l3",
            Self::MemBw => "membw",
            Self::MemCap => "memcap",
            Self::Cpu => "cpu",
            Self::IoBw => "iobw",
            Self::IoOps => "iops",
        }
    }

    pub fn resource(&self) -> &'static str {
        match self {
            Self::L1d => "L1 data cache",
            Self::L2 => "L2 cache",
            Self::L3 => "LLC (L3)",
            Self::MemBw => "Memory bandwidth",
            Self::MemCap => "Memory capacity",
            Self::Cpu => "CPU integer units",
            Self::IoBw => "Storage bandwidth",
            Self::IoOps => "Storage IOPS",
        }
    }

    pub fn backend(&self) -> SoiBackend {
        match self {
            Self::IoBw => SoiBackend::External(IOBW_FIO_TEMPLATE),
            Self::IoOps => SoiBackend::External(IOOPS_FIO_TEMPLATE),
            _ => SoiBackend::Internal,
        }
    }
}

impl fmt::Display for SoiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Parse a comma-separated SoI list or "all".
pub fn parse_soi_list(s: &str) -> Result<Vec<SoiType>, String> {
    if s.to_lowercase() == "all" {
        return Ok(SoiType::all());
    }
    let mut types = Vec::new();
    for part in s.split(',') {
        let trimmed = part.trim();
        match SoiType::from_str(trimmed) {
            Some(t) => types.push(t),
            None => return Err(format!("Unknown SoI type: '{}'. Valid: l1d, l2, l3, membw, memcap, cpu, iobw, iops, all", trimmed)),
        }
    }
    if types.is_empty() {
        return Err("No SoI types specified".into());
    }
    Ok(types)
}

/// Detected cache sizes from sysfs.
#[derive(Debug, Clone, Copy)]
pub struct CacheSizes {
    pub l1d: usize,
    pub l2: usize,
    pub l3: usize,
}

/// Detect cache sizes from sysfs. Falls back to defaults if not available.
pub fn detect_cache_sizes() -> CacheSizes {
    let l1d = read_cache_size(0)
        .unwrap_or(SOI_FALLBACK_L1D_BYTES);
    let l2 = read_cache_size(2)
        .unwrap_or(SOI_FALLBACK_L2_BYTES);
    let l3 = read_cache_size(3)
        .unwrap_or(SOI_FALLBACK_L3_BYTES);

    CacheSizes { l1d, l2, l3 }
}

/// Read cache size from /sys/devices/system/cpu/cpu0/cache/index{N}/size.
/// index0 = L1i, index1 = L1d, index2 = L2, index3 = L3.
fn read_cache_size(index: usize) -> Option<usize> {
    let path = format!("/sys/devices/system/cpu/cpu0/cache/index{}/size", index);
    let content = std::fs::read_to_string(&path).ok()?;
    parse_size_suffix(content.trim())
}

/// Parse a size string with K/M/G suffix (e.g., "32K", "8M").
fn parse_size_suffix(s: &str) -> Option<usize> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, multiplier) = if s.ends_with('K') || s.ends_with('k') {
        (&s[..s.len()-1], 1024)
    } else if s.ends_with('M') || s.ends_with('m') {
        (&s[..s.len()-1], 1024 * 1024)
    } else if s.ends_with('G') || s.ends_with('g') {
        (&s[..s.len()-1], 1024 * 1024 * 1024)
    } else {
        (s, 1)
    };
    num_str.parse::<usize>().ok().map(|n| n * multiplier)
}

/// Return the buffer size for a given SoI type.
pub fn soi_buffer_size(soi: SoiType, cache_sizes: &CacheSizes, soi_count: usize) -> usize {
    match soi {
        SoiType::L1d => cache_sizes.l1d,
        SoiType::L2 => cache_sizes.l2,
        SoiType::L3 => cache_sizes.l3,
        SoiType::MemBw => SOI_MEMBW_BUFFER_BYTES,
        SoiType::MemCap => {
            let total = read_cgroup_mem_limit()
                .unwrap_or_else(read_proc_meminfo_total);
            let per_worker = ((total as f64 * SOI_MEMCAP_FRACTION) as usize) / soi_count.max(1);
            // Minimum 1MB per worker to be useful
            per_worker.max(1024 * 1024)
        }
        SoiType::Cpu => 0, // no buffer needed
        SoiType::IoBw => SOI_IOBW_BLOCK_BYTES,
        SoiType::IoOps => SOI_IOOPS_BLOCK_BYTES,
    }
}

fn read_cgroup_mem_limit() -> Option<usize> {
    let content = std::fs::read_to_string("/sys/fs/cgroup/memory.max").ok()?;
    let trimmed = content.trim();
    if trimmed == "max" {
        return None;
    }
    trimmed.parse().ok()
}

fn read_proc_meminfo_total() -> usize {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let Ok(kb) = parts[1].parse::<usize>() {
                    return kb * 1024;
                }
            }
        }
    }
    // Fallback: 8GB
    8 * 1024 * 1024 * 1024
}

// --- SoI work functions ---

/// Cache pressure SoI: memcpy half ← half within a cache-sized buffer.
/// Used by L1d, L2, L3 SoIs.
fn soi_cache_work(deadline_ns: &AtomicU64, counter: &AtomicU64, buffer: &mut [u8]) {
    let len = buffer.len();
    if len < 2 { return; }
    let half = len / 2;
    let mut local_ops = 0u64;

    loop {
        let d = deadline_ns.load(Ordering::Relaxed);
        if d != 0 && now_ns() >= d {
            break;
        }

        // Copy second half to first half
        let ptr = buffer.as_mut_ptr();
        unsafe {
            std::ptr::copy_nonoverlapping(ptr.add(half), ptr, half);
        }

        local_ops += 1;
        if local_ops >= BATCH_FLUSH_THRESHOLD {
            counter.fetch_add(local_ops, Ordering::Relaxed);
            local_ops = 0;
        }
    }

    if local_ops > 0 {
        counter.fetch_add(local_ops, Ordering::Relaxed);
    }
}

/// Memory bandwidth SoI: streaming scalar multiply over f64 array.
fn soi_membw_work(deadline_ns: &AtomicU64, counter: &AtomicU64, buffer: &mut [u8]) {
    let f64_count = buffer.len() / 8;
    if f64_count < 2 { return; }
    let data = unsafe { std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut f64, f64_count) };

    // Initialize with non-zero values
    for (i, v) in data.iter_mut().enumerate() {
        *v = (i as f64) * 0.001 + 1.0;
    }

    let mut local_ops = 0u64;
    let scale = 1.0000001f64;

    loop {
        let d = deadline_ns.load(Ordering::Relaxed);
        if d != 0 && now_ns() >= d {
            break;
        }

        // Forward pass: multiply
        for v in data.iter_mut() {
            *v *= scale;
        }
        // Backward pass: divide (keeps values bounded)
        for v in data.iter_mut() {
            *v /= scale;
        }

        local_ops += 1;
        if local_ops >= BATCH_FLUSH_THRESHOLD {
            counter.fetch_add(local_ops, Ordering::Relaxed);
            local_ops = 0;
        }
    }

    std::hint::black_box(&data[0]);
    if local_ops > 0 {
        counter.fetch_add(local_ops, Ordering::Relaxed);
    }
}

/// Memory capacity SoI: memcpy over a huge buffer (same pattern as cache SoI).
fn soi_memcap_work(deadline_ns: &AtomicU64, counter: &AtomicU64, buffer: &mut [u8]) {
    soi_cache_work(deadline_ns, counter, buffer);
}

/// Pure CPU SoI: busy-spin integer ops.
fn soi_cpu_work(deadline_ns: &AtomicU64, counter: &AtomicU64) {
    let mut local_ops = 0u64;
    let mut hash = 0u64;

    loop {
        let d = deadline_ns.load(Ordering::Relaxed);
        if d != 0 && now_ns() >= d {
            break;
        }

        for i in 0..SOI_CPU_BATCH_ITERS {
            hash = hash.wrapping_mul(HASH_MUL_PRIMARY).wrapping_add(i);
            hash = hash.wrapping_mul(HASH_MUL_SECONDARY).wrapping_add(hash >> 17);
        }

        local_ops += 1;
        if local_ops >= BATCH_FLUSH_THRESHOLD {
            counter.fetch_add(local_ops, Ordering::Relaxed);
            local_ops = 0;
        }
    }

    std::hint::black_box(hash);
    if local_ops > 0 {
        counter.fetch_add(local_ops, Ordering::Relaxed);
    }
}

/// IO SoI: O_DIRECT|O_SYNC pwrite at given block size.
/// `random` = true for IOPs (random offsets), false for bandwidth (sequential).
fn soi_io_work(deadline_ns: &AtomicU64, counter: &AtomicU64, worker_id: usize, block_size: usize, random: bool) {
    let path = format!("/tmp/saturator_soi_{}", worker_id);
    let file_size = SOI_IO_FILE_SIZE_BYTES.max(256 * block_size);

    // Allocate aligned buffer for O_DIRECT
    let buf = unsafe {
        let mut ptr: *mut libc::c_void = std::ptr::null_mut();
        let ret = libc::posix_memalign(&mut ptr, 4096, block_size);
        assert_eq!(ret, 0, "posix_memalign failed for SoI IO buffer");
        std::ptr::write_bytes(ptr as *mut u8, 0xAA, block_size);
        std::slice::from_raw_parts(ptr as *const u8, block_size)
    };

    let c_path = CString::new(path.as_str()).unwrap();
    let flags = libc::O_CREAT | libc::O_RDWR | libc::O_SYNC | libc::O_DIRECT;
    let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o600) };
    assert!(fd >= 0, "SoI IO: open({}) failed: {}", path, std::io::Error::last_os_error());

    let ret = unsafe { libc::posix_fallocate(fd, 0, file_size as libc::off_t) };
    assert_eq!(ret, 0, "SoI IO: posix_fallocate failed");

    let max_blocks = file_size / block_size;
    let mut offset: usize = 0;
    let mut local_ops = 0u64;
    let mut rng_state = worker_id as u64 + RNG_SEED_OFFSET;

    loop {
        let d = deadline_ns.load(Ordering::Relaxed);
        if d != 0 && now_ns() >= d {
            break;
        }

        if random {
            rng_state = rng_state.wrapping_mul(PCG_MULTIPLIER).wrapping_add(1);
            let block_idx = (rng_state >> 32) as usize % max_blocks;
            offset = block_idx * block_size;
        }

        let _written = unsafe {
            libc::pwrite(fd, buf.as_ptr() as *const libc::c_void, block_size, offset as libc::off_t)
        };

        if !random {
            offset = (offset + block_size) % file_size;
        }

        local_ops += 1;
        if local_ops >= BATCH_FLUSH_THRESHOLD {
            counter.fetch_add(local_ops, Ordering::Relaxed);
            local_ops = 0;
        }
    }

    if local_ops > 0 {
        counter.fetch_add(local_ops, Ordering::Relaxed);
    }

    unsafe { libc::close(fd); }
    let _ = std::fs::remove_file(&path);
}

// --- Child process entry point ---

/// SoI worker child process entry point.
/// Opens shared memory, allocates appropriate buffer, runs the SoI work loop.
pub fn run_soi_worker_process(
    shm_name: &str,
    worker_id: usize,
    max_workers: usize,
    soi_type: SoiType,
    buffer_size: usize,
) {
    let region = open_shared_region(shm_name, max_workers);
    let region_ref = unsafe { &*region };

    // SoI workers use the same per-worker counter slots.
    // Cache/CPU/Mem SoIs write to cpu_ops; IO SoIs write to io_ops.
    let (wk_cpu, wk_io, _wk_sleep, _wk_errors) = unsafe { worker_counters(region, worker_id) };

    match soi_type {
        SoiType::L1d | SoiType::L2 | SoiType::L3 => {
            let mut buffer = alloc_mmap_buffer(buffer_size);
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_cache_work(&region_ref.deadline_ns, wk_cpu, &mut buffer);
            free_mmap_buffer(&mut buffer);
        }
        SoiType::MemBw => {
            let mut buffer = alloc_mmap_buffer(buffer_size);
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_membw_work(&region_ref.deadline_ns, wk_cpu, &mut buffer);
            free_mmap_buffer(&mut buffer);
        }
        SoiType::MemCap => {
            let mut buffer = alloc_mmap_buffer(buffer_size);
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_memcap_work(&region_ref.deadline_ns, wk_cpu, &mut buffer);
            free_mmap_buffer(&mut buffer);
        }
        SoiType::Cpu => {
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_cpu_work(&region_ref.deadline_ns, wk_cpu);
        }
        SoiType::IoBw => {
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_io_work(&region_ref.deadline_ns, wk_io, worker_id, SOI_IOBW_BLOCK_BYTES, false);
        }
        SoiType::IoOps => {
            region_ref.ready_count.fetch_add(1, Ordering::Relaxed);
            soi_io_work(&region_ref.deadline_ns, wk_io, worker_id, SOI_IOOPS_BLOCK_BYTES, true);
        }
    }
}

/// Allocate a buffer via mmap (anonymous, private) — better for large allocations
/// as it avoids heap fragmentation and is returned to the OS on munmap.
fn alloc_mmap_buffer(size: usize) -> Vec<u8> {
    // For SoI workloads, we want the buffer to be fully resident.
    // Use a Vec and touch every page.
    let mut buf = vec![0u8; size];
    // Touch every page to fault it in
    let page_size = 4096;
    for i in (0..size).step_by(page_size) {
        buf[i] = (i & 0xFF) as u8;
    }
    buf
}

fn free_mmap_buffer(_buf: &mut Vec<u8>) {
    // Vec drops automatically; this is a no-op placeholder for symmetry.
}
