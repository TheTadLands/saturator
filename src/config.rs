use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

/// Configuration for a single thread or group of threads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadConfig {
    /// Descriptive name for this thread group
    pub name: String,
    
    /// Percentage of operations that should be I/O (0.0 = pure CPU, 1.0 = pure I/O)
    pub io_percentage: f64,
    
    /// Number of threads to spawn with this configuration
    #[serde(default = "default_count")]
    pub count: usize,
}

fn default_count() -> usize {
    1
}

/// Complete experiment configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentConfig {
    /// Name/description of the experiment
    #[serde(default)]
    pub name: String,
    
    /// How long to run the experiment in seconds
    #[serde(default = "default_duration")]
    pub duration_secs: u64,
    
    /// Thread configurations
    pub threads: Vec<ThreadConfig>,
}

fn default_duration() -> u64 {
    60
}

impl ExperimentConfig {
    /// Load configuration from a JSON file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::Io(e))?;
        
        let config: ExperimentConfig = serde_json::from_str(&content)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        
        config.validate()?;
        Ok(config)
    }
    
    /// Save configuration to a JSON file
    pub fn to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        self.validate()?;
        
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;
        
        fs::write(path.as_ref(), json)
            .map_err(|e| ConfigError::Io(e))?;
        
        Ok(())
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.threads.is_empty() {
            return Err(ConfigError::Validation("Must have at least one thread configuration".to_string()));
        }
        
        for (i, thread) in self.threads.iter().enumerate() {
            if thread.io_percentage < 0.0 || thread.io_percentage > 1.0 {
                return Err(ConfigError::Validation(
                    format!("Thread '{}' (index {}): io_percentage must be between 0.0 and 1.0", 
                            thread.name, i)
                ));
            }
            
            if thread.count == 0 {
                return Err(ConfigError::Validation(
                    format!("Thread '{}' (index {}): count must be at least 1", 
                            thread.name, i)
                ));
            }
        }
        
        if self.duration_secs == 0 {
            return Err(ConfigError::Validation("duration_secs must be greater than 0".to_string()));
        }
        
        Ok(())
    }
    
    /// Get total number of threads that will be spawned
    pub fn total_thread_count(&self) -> usize {
        self.threads.iter().map(|t| t.count).sum()
    }
}

/// Error types for configuration operations
#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "I/O error: {}", e),
            ConfigError::Parse(e) => write!(f, "Parse error: {}", e),
            ConfigError::Validation(e) => write!(f, "Validation error: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Generate example configuration files
pub mod examples {
    use super::*;
    
    /// Pure CPU baseline experiment
    pub fn cpu_baseline() -> ExperimentConfig {
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        
        ExperimentConfig {
            name: "CPU Baseline - Pure CPU Saturation".to_string(),
            duration_secs: 30,
            threads: vec![
                ThreadConfig {
                    name: "CPU-Only".to_string(),
                    io_percentage: 0.0,
                    count: num_cpus,
                }
            ],
            cpu_iterations: None,
            io_size: None,
        }
    }
    
    /// CPU saturation plus additional CPU threads
    pub fn cpu_oversubscribed() -> ExperimentConfig {
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        
        ExperimentConfig {
            name: "CPU Oversubscribed - Test Contention".to_string(),
            duration_secs: 30,
            threads: vec![
                ThreadConfig {
                    name: "CPU-Baseline".to_string(),
                    io_percentage: 0.0,
                    count: num_cpus,
                },
                ThreadConfig {
                    name: "CPU-Additional".to_string(),
                    io_percentage: 0.0,
                    count: num_cpus / 2,
                }
            ],
            cpu_iterations: None,
            io_size: None,
        }
    }
    
    /// CPU saturation with I/O threads to find slack
    pub fn cpu_with_io() -> ExperimentConfig {
        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        
        ExperimentConfig {
            name: "CPU + I/O - Find System Slack".to_string(),
            duration_secs: 30,
            threads: vec![
                ThreadConfig {
                    name: "CPU-Workers".to_string(),
                    io_percentage: 0.0,
                    count: num_cpus,
                },
                ThreadConfig {
                    name: "IO-Workers".to_string(),
                    io_percentage: 1.0,
                    count: num_cpus,
                }
            ],
            cpu_iterations: None,
            io_size: None,
        }
    }
    
    /// Mixed workload with varying I/O percentages
    pub fn mixed_workload() -> ExperimentConfig {
        ExperimentConfig {
            name: "Mixed Workload - Various I/O Ratios".to_string(),
            duration_secs: 30,
            threads: vec![
                ThreadConfig {
                    name: "Pure-CPU".to_string(),
                    io_percentage: 0.0,
                    count: 2,
                },
                ThreadConfig {
                    name: "CPU-Heavy".to_string(),
                    io_percentage: 0.2,
                    count: 2,
                },
                ThreadConfig {
                    name: "Balanced".to_string(),
                    io_percentage: 0.5,
                    count: 2,
                },
                ThreadConfig {
                    name: "IO-Heavy".to_string(),
                    io_percentage: 0.8,
                    count: 2,
                },
                ThreadConfig {
                    name: "Pure-IO".to_string(),
                    io_percentage: 1.0,
                    count: 2,
                }
            ],
            cpu_iterations: None,
            io_size: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_validation_io_percentage() {
        let mut config = examples::cpu_baseline();
        config.threads[0].io_percentage = 1.5;
        assert!(config.validate().is_err());
        
        config.threads[0].io_percentage = -0.1;
        assert!(config.validate().is_err());
        
        config.threads[0].io_percentage = 0.5;
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_total_thread_count() {
        let config = examples::mixed_workload();
        assert_eq!(config.total_thread_count(), 10);
    }
    
    #[test]
    fn test_serialization() {
        let config = examples::cpu_baseline();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ExperimentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.name, parsed.name);
        assert_eq!(config.threads.len(), parsed.threads.len());
    }
}