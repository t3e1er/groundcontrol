//! Process memory usage tracker using `sysinfo`.

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Snapshot of memory metrics during an execution stage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryMetrics {
    /// Resident Set Size at start of stage (bytes).
    pub start_rss_bytes: u64,
    /// Peak Resident Set Size observed during stage (bytes).
    pub peak_rss_bytes: u64,
    /// Resident Set Size at end of stage (bytes).
    pub end_rss_bytes: u64,
    /// Delta increase from start to end (bytes).
    pub delta_rss_bytes: u64,
}

impl MemoryMetrics {
    /// Format bytes as human-readable megabytes.
    pub fn peak_mb(&self) -> f64 {
        self.peak_rss_bytes as f64 / (1024.0 * 1024.0)
    }

    /// Format delta bytes as human-readable megabytes.
    pub fn delta_mb(&self) -> f64 {
        self.delta_rss_bytes as f64 / (1024.0 * 1024.0)
    }
}

/// Active memory tracker that samples current process memory.
pub struct MemoryTracker {
    system: System,
    pid: Pid,
    start_rss_bytes: u64,
    peak_rss_bytes: u64,
}

impl MemoryTracker {
    /// Start tracking memory for the current process.
    pub fn start() -> Self {
        let mut system = System::new();
        let pid = Pid::from_u32(std::process::id());
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let rss = system.process(pid).map(|p| p.memory()).unwrap_or(0);
        Self { system, pid, start_rss_bytes: rss, peak_rss_bytes: rss }
    }

    /// Sample current memory and update peak RSS if current exceeds it.
    pub fn sample(&mut self) -> u64 {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        let current = self.system.process(self.pid).map(|p| p.memory()).unwrap_or(0);
        if current > self.peak_rss_bytes {
            self.peak_rss_bytes = current;
        }
        current
    }

    /// Complete tracking and return summary metrics.
    pub fn finish(mut self) -> MemoryMetrics {
        let current = self.sample();
        MemoryMetrics {
            start_rss_bytes: self.start_rss_bytes,
            peak_rss_bytes: self.peak_rss_bytes,
            end_rss_bytes: current,
            delta_rss_bytes: current.saturating_sub(self.start_rss_bytes),
        }
    }
}
