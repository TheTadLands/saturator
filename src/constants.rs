/// PCG LCG multiplier (Knuth's constant for 64-bit linear congruential generator).
pub const PCG_MULTIPLIER: u64 = 6364136223846793005;

/// Default RNG seed offset, added to worker/thread ID for deterministic per-worker sequences.
pub const RNG_SEED_OFFSET: u64 = 12345;

/// Bytes to stride through the CPU work buffer per iteration.
pub const CPU_BUFFER_STRIDE: usize = 1000;

/// Extra hash rounds per CPU work iteration (unroll factor for more compute per op).
pub const CPU_HASH_UNROLL: u64 = 50;

/// Primary hash multiplier (FNV-style folding).
pub const HASH_MUL_PRIMARY: u64 = 31;

/// Secondary hash multiplier for extra hash rounds.
pub const HASH_MUL_SECONDARY: u64 = 7;

/// Flush local ops counters to shared atomics every N operations.
pub const BATCH_FLUSH_THRESHOLD: u64 = 100;

/// Number of I/O samples during calibration timing.
pub const CALIBRATION_IO_SAMPLES: u128 = 100;

/// Number of CPU samples during the coarse scaling phase of calibration.
pub const CALIBRATION_CPU_SCALING_SAMPLES: u128 = 200;

/// Number of CPU samples during the binary search fine-tuning phase.
pub const CALIBRATION_CPU_SEARCH_SAMPLES: u128 = 500;

/// Number of CPU samples for the final verification measurement.
pub const CALIBRATION_CPU_FINAL_SAMPLES: u128 = 1000;

/// Calibration tolerance: accept CPU timing within +/- this percentage of IO timing.
pub const CALIBRATION_TOLERANCE_PCT: u128 = 2;

/// Maximum binary search iterations during CPU calibration.
pub const CALIBRATION_MAX_SEARCH_ITERS: usize = 40;

/// Maximum CPU iteration count before giving up during coarse scaling.
pub const CALIBRATION_MAX_CPU_ITERS: usize = 1_000_000;

/// Number of calibration passes; median is taken across passes.
pub const CALIBRATION_PASSES: usize = 3;

/// Bytes per worker in shared memory layout (3 x AtomicU64: cpu_ops, io_ops, sleep_ops).
/// Padded to 64 bytes (one cache line) to eliminate false sharing between adjacent workers.
pub const SHM_BYTES_PER_WORKER: usize = 64;

/// Page size for shared memory alignment (rounds up to page boundary).
pub const PAGE_SIZE: usize = 4096;

/// Number of intensity steps when sweeping probe intensity from 0.0 to 1.0.
pub const INTENSITY_SWEEP_STEPS: u32 = 20;

/// Poll interval in milliseconds when waiting for child processes to signal ready.
pub const READY_POLL_INTERVAL_MS: u64 = 10;
