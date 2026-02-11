use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::io::{Read, Write, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;

#[derive(Debug, Clone)]
pub struct WorkloadConfig {
    pub name: String,
    pub io_perc: f64,
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

pub fn calibrate_operations() -> (usize, usize) {
    let result = calibrate_operations_full();
    (result.cpu_iterations, result.io_iterations)
}

pub fn calibrate_operations_full() -> CalibrationResult {
    println!("  Running calibration (3 passes, targeting <5% gap)...");
    
    let mut results = Vec::new();
    
    for pass in 0..3 {
        let (cpu_iters, cpu_us, io_iters, io_us) = calibrate_single_pass();
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

fn calibrate_single_pass() -> (usize, u128, usize, u128) {
    let target_us: u128 = 50;
    
    // Calibrate I/O first (coarser granularity)
    let (io_iters, io_actual_us) = {
        let path = "/tmp/saturator_calibrate";
        let _ = std::fs::write(path, vec![0u8; 4096]);
        
        let mut iters = 1;
        let mut actual_us = 0u128;
        
        loop {
            let start = std::time::Instant::now();
            let samples = 100;
            for _ in 0..samples {
                do_io_work_counted(path, iters);
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
    
    // Now calibrate CPU to match I/O timing precisely (within 5%)
    let (cpu_iters, cpu_actual_us) = {
        let buffer: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
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
        
        // Fine-tune with binary search to get within 5%
        let mut low = iters / 2;
        let mut high = iters * 2;
        let lower_bound = target_cpu_us * 95 / 100;
        let upper_bound = target_cpu_us * 105 / 100;
        
        // More iterations for tighter convergence
        for _ in 0..20 {
            let mid = (low + high) / 2;
            if mid == low || mid == high {
                break;
            }
            
            let start = std::time::Instant::now();
            let samples = 200;
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
        let samples = 500;
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
    config: WorkloadConfig,
    cpu_iterations: usize,
    io_iterations: usize,
    total_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
) {
    let cpu_buffer: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_{}", thread_id);
    
    // Create a small file for I/O
    if config.io_perc > 0.0 {
        let _ = std::fs::write(&io_path, vec![0u8; 4096]);
    }
    
    let mut local_ops = 0u64;
    let mut rng_state = thread_id as u64 + 12345;
    
    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;
        
        if rand_f64 < config.io_perc {
            do_io_work_counted(&io_path, io_iterations);
        } else {
            do_cpu_work(&cpu_buffer, cpu_iterations);
        }
        
        local_ops += 1;
        
        if local_ops % 100 == 0 {
            total_ops.fetch_add(100, Ordering::Relaxed);
            local_ops = 0;
        }
    }
    
    if local_ops > 0 {
        total_ops.fetch_add(local_ops, Ordering::Relaxed);
    }
    
    // Always try to clean up
    let _ = std::fs::remove_file(&io_path);
}

fn do_cpu_work(buffer: &[u8], iterations: usize) -> u64 {
    let mut hash = 0u64;
    
    for _ in 0..iterations {
        hash = buffer.iter().take(1000).fold(hash, |acc, &b| {
            acc.wrapping_mul(31).wrapping_add(b as u64)
        });
        
        for i in 0..50 {
            hash = hash.wrapping_add(i).wrapping_mul(7);
        }
    }
    
    std::hint::black_box(hash)
}

fn do_io_work_counted(path: &str, iterations: usize) {
    for _ in 0..iterations {
        // Use O_SYNC for true blocking I/O - waits for data to hit disk
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_SYNC)
            .open(path)
        {
            let mut buf = [0u8; 512];
            let _ = file.read(&mut buf);
            let _ = file.seek(SeekFrom::Start(0));
            let _ = file.write_all(&buf);
            // O_SYNC ensures write is complete before returning
        }
    }
}

// Keep old do_io_work for compatibility but it's no longer used
fn do_io_work(path: &str, _size: usize) {
    do_io_work_counted(path, 1);
}

// Thread-local buffer for cpu_work to match run_saturator behavior
thread_local! {
    static CPU_BUFFER: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
}

pub fn cpu_work(iterations: usize) {
    CPU_BUFFER.with(|buffer| {
        do_cpu_work(buffer, iterations);
    });
}

pub fn io_work(iterations: usize) {
    io_work_with_id(0, iterations);
}

pub fn io_work_with_id(thread_id: usize, iterations: usize) {
    let path = format!("/tmp/saturator_io_{}", thread_id);
    
    // Create file once if it doesn't exist
    if !std::path::Path::new(&path).exists() {
        let _ = std::fs::write(&path, vec![0u8; 4096]);
    }
    
    do_io_work_counted(&path, iterations);
}