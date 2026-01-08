use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn monitor_throughput(total_ops: Arc<AtomicU64>, running: Arc<AtomicBool>) {
    let mut last_ops = 0u64;
    let start_time = Instant::now();
    let mut last_time = start_time;
    
    println!("Time(s)\tOps/sec\t\tTotal Ops");
    println!("----------------------------------------");
    
    while running.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
        
        let now = Instant::now();
        let current_ops = total_ops.load(Ordering::Relaxed);
        let elapsed = now.duration_since(last_time).as_secs_f64();
        
        let ops_delta = current_ops - last_ops;
        let throughput = ops_delta as f64 / elapsed;
        
        let total_elapsed = now.duration_since(start_time).as_secs();
        println!("{}\t{:.0}\t\t{}", total_elapsed, throughput, current_ops);
        
        last_ops = current_ops;
        last_time = now;
    }
}