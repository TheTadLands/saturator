use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::io::{Read, Write, Seek, SeekFrom};

#[derive(Debug, Clone)]
pub struct WorkloadConfig {
    pub name: String,
    pub io_perc: f64,
}

pub fn calibrate_operations() -> (usize, usize) {
    let target_us = 50;
    
    // Calibrate CPU
    let cpu_iters = {
        let buffer: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        let mut best = 1000;
        
        for mult in [1, 10, 100, 1000, 10000] {
            let start = Instant::now();
            for _ in 0..10 {
                do_cpu_work(&buffer, mult);
            }
            let avg_us = start.elapsed().as_micros() / 10;
            
            if avg_us >= target_us as u128 {
                best = mult;
                break;
            }
        }
        best
    };
    
    // Calibrate I/O
    let io_size = {
        let path = "/tmp/saturator_calibrate";
        std::fs::write(path, vec![0u8; 1024 * 1024]).unwrap();
        
        let mut best = 4096;
        for size in [512, 1024, 4096, 8192, 16384, 32768] {
            let start = Instant::now();
            for _ in 0..10 {
                do_io_work(path, size);
            }
            let avg_us = start.elapsed().as_micros() / 10;
            
            if avg_us >= target_us as u128 {
                best = size;
                break;
            }
        }
        
        std::fs::remove_file(path).unwrap();
        best
    };
    
    (cpu_iters, io_size)
}

pub fn run_saturator(
    thread_id: usize,
    config: WorkloadConfig,
    cpu_iterations: usize,
    io_size: usize,
    total_ops: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
) {
    let cpu_buffer: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
    let io_path = format!("/tmp/saturator_{}", thread_id);
    
    if config.io_perc > 0.0 {
        std::fs::write(&io_path, vec![0u8; io_size * 1000]).unwrap();
    }
    
    let mut local_ops = 0u64;
    let mut rng_state = thread_id as u64 + 12345;
    
    while running.load(Ordering::Relaxed) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let rand_f64 = (rng_state >> 32) as f64 / u32::MAX as f64;
        
        if rand_f64 < config.io_perc {
            do_io_work(&io_path, io_size);
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
    
    if config.io_perc > 0.0 {
        let _ = std::fs::remove_file(&io_path);
    }
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

fn do_io_work(path: &str, size: usize) {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    
    let offset = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() % 500000) as u64;
    
    file.seek(SeekFrom::Start(offset)).unwrap();
    
    let mut buf = vec![0u8; size];
    let _ = file.read(&mut buf);
    file.seek(SeekFrom::Start(offset)).unwrap();
    let _ = file.write_all(&buf);
    file.sync_data().unwrap();
}