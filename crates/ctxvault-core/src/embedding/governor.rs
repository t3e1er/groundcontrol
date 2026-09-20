#[cfg(target_os = "windows")]
use std::sync::atomic::AtomicUsize;
#[cfg(target_os = "windows")]
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
#[cfg(target_os = "windows")]
use std::time::{Duration, Instant};

/// Helper struct representing a detected display adapter on Windows.
#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
pub(crate) struct DetectedAdapter {
    pub(crate) device_id: i32,
    pub(crate) name: String,
    pub(crate) vram_mb: usize,
    pub(crate) is_discrete: bool,
}

#[cfg(target_os = "windows")]
pub(crate) fn is_discrete_gpu_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    (lower.contains("nvidia")
        || lower.contains("geforce")
        || lower.contains("rtx")
        || lower.contains("gtx")
        || lower.contains("radeon")
        || lower.contains("arc"))
        && !lower.contains("basic")
        && !lower.contains("remote")
        && !lower.contains("virtual")
}

/// Discover available Windows display adapters using native Display Driver Registry keys
/// (matching true DXGI adapter numbering and 64-bit VRAM capacity) with WMI fallback.
#[cfg(target_os = "windows")]
pub(crate) fn discover_windows_gpu_adapters() -> Vec<DetectedAdapter> {
    // Strategy A: Query display driver registry keys.
    // DXGI adapter indexes correspond directly to driver instance keys (0000 -> 0, etc.).
    // HardwareInformation.qwMemorySize provides un-truncated 64-bit VRAM size.
    if let Ok(output) = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-ItemProperty 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\000*' | \
             Where-Object { $_.DriverDesc -and $_.DriverDesc -notmatch 'Basic|Remote|Virtual' } | \
             Select-Object @{N='Index'; E={[int]$_.PSChildName}}, DriverDesc, @{N='MemoryBytes'; E={if ($_.'HardwareInformation.qwMemorySize') { [uint64]$_.'HardwareInformation.qwMemorySize' } else { 0 }}} | \
             ConvertTo-Json -Compress",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout) {
                let items = if let Some(arr) = val.as_array() {
                    arr.clone()
                } else if val.is_object() {
                    vec![val]
                } else {
                    Vec::new()
                };

                let mut adapters = Vec::new();
                for item in items {
                    let idx = item.get("Index").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    let name = item
                        .get("DriverDesc")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let mem_bytes = item.get("MemoryBytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    let vram_mb = (mem_bytes / (1024 * 1024)) as usize;
                    let is_discrete = is_discrete_gpu_name(&name);

                    adapters.push(DetectedAdapter {
                        device_id: idx,
                        name,
                        vram_mb,
                        is_discrete,
                    });
                }

                if !adapters.is_empty() {
                    return adapters;
                }
            }
        }
    }

    // Strategy B: Fallback to WMI Win32_VideoController
    if let Ok(output) = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name, AdapterRAM | ConvertTo-Json -Compress",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout) {
                let items = if let Some(arr) = val.as_array() {
                    arr.clone()
                } else if val.is_object() {
                    vec![val]
                } else {
                    Vec::new()
                };

                let mut adapters = Vec::new();
                for (wmi_idx, item) in items.iter().enumerate() {
                    let name = item
                        .get("Name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let ram = item.get("AdapterRAM").and_then(|v| v.as_u64()).unwrap_or(0);
                    let vram_mb = (ram / (1024 * 1024)) as usize;
                    let is_discrete = is_discrete_gpu_name(&name);

                    // Discrete GPUs on desktop Windows are almost universally DXGI Adapter 0
                    let device_id = if is_discrete { 0 } else { wmi_idx as i32 };

                    adapters.push(DetectedAdapter {
                        device_id,
                        name,
                        vram_mb,
                        is_discrete,
                    });
                }

                if !adapters.is_empty() {
                    return adapters;
                }
            }
        }
    }

    vec![DetectedAdapter {
        device_id: 0,
        name: "Default Graphics Adapter".to_string(),
        vram_mb: 1024,
        is_discrete: false,
    }]
}

#[cfg(target_os = "windows")]
static DETECTED_GPU: std::sync::OnceLock<(i32, usize)> = std::sync::OnceLock::new();

/// Automatically select the most appropriate DirectML GPU device ID across all Windows systems.
#[cfg(target_os = "windows")]
pub fn select_directml_device_id() -> i32 {
    // 1. Explicit override via CTX_DEVICE_ID
    if let Ok(val) = std::env::var("CTX_DEVICE_ID") {
        if let Ok(id) = val.parse::<i32>() {
            tracing::info!(device_id = id, "DirectML GPU device ID selected via CTX_DEVICE_ID");
            return id;
        }
    }

    let (device_id, _) = *DETECTED_GPU.get_or_init(|| {
        let mut adapters = discover_windows_gpu_adapters();

        for adapter in &adapters {
            tracing::debug!(
                device_id = adapter.device_id,
                name = %adapter.name,
                vram_mb = adapter.vram_mb,
                is_discrete = adapter.is_discrete,
                "Detected GPU adapter"
            );
        }

        adapters.sort_by(|a, b| {
            b.is_discrete
                .cmp(&a.is_discrete)
                .then_with(|| b.vram_mb.cmp(&a.vram_mb))
                .then_with(|| a.device_id.cmp(&b.device_id))
        });

        if let Some(best) = adapters.first() {
            tracing::info!(
                device_id = best.device_id,
                name = %best.name,
                vram_mb = best.vram_mb,
                "Automatically selected high-performance DirectML GPU adapter"
            );
            (best.device_id, best.vram_mb)
        } else {
            (0, 1024)
        }
    });

    device_id
}

/// Ordered list of candidate DirectML device IDs to configure for hardware acceleration.
#[cfg(target_os = "windows")]
pub fn directml_device_candidates() -> Vec<i32> {
    if let Ok(val) = std::env::var("CTX_DEVICE_ID") {
        if let Ok(id) = val.parse::<i32>() {
            return vec![id];
        }
    }

    let detected = select_directml_device_id();
    let mut candidates = vec![detected];
    if !candidates.contains(&0) {
        candidates.push(0);
    }
    if !candidates.contains(&1) {
        candidates.push(1);
    }
    candidates
}

/// Detect dedicated VRAM in megabytes for the primary GPU adapter on Windows.
#[cfg(target_os = "windows")]
pub fn detect_gpu_vram_mb() -> usize {
    let _ = select_directml_device_id();
    DETECTED_GPU.get().map(|&(_, vram)| vram).unwrap_or(1024)
}

/// Trait defining device memory introspection, target execution slicing, and AIMD batch scaling.
pub trait HardwareGovernor: Send + Sync {
    /// Query real-time available memory headroom on the target compute device (bytes).
    fn available_memory_bytes(&self) -> usize;

    /// Total device memory capacity (bytes).
    fn total_memory_bytes(&self) -> usize;

    /// Compute the adaptive batch size for a given sequence length,
    /// informed by the measured latency of the previous dispatch.
    fn compute_adaptive_batch(&self, seq_len: usize, last_dispatch_ms: u64) -> usize;

    /// Target execution slice duration in milliseconds.
    /// Dispatches should aim for this duration to balance throughput vs. desktop responsiveness.
    fn target_slice_ms(&self) -> u64;

    /// Report the current compute provider name for logging.
    fn provider_name(&self) -> &str;
}

/// Additive Increase / Multiplicative Decrease (AIMD) controller for dynamically scaling batch sizes.
#[derive(Debug, Clone)]
pub struct AimdController {
    /// Multiplier scale factor, clamped between 0.2 and 4.0 (starts at 1.0).
    pub scale: f64,
    /// Exponential moving average of dispatch latency (ms).
    pub ema_latency_ms: f64,
    /// Target execution slice duration in milliseconds.
    pub target_slice_ms: u64,
    /// Number of dispatches observed.
    pub sample_count: u64,
}

impl AimdController {
    /// Create a new AIMD controller targeting the specified slice duration in milliseconds.
    pub fn new(target_slice_ms: u64) -> Self {
        Self {
            scale: 1.0,
            ema_latency_ms: target_slice_ms as f64,
            target_slice_ms,
            sample_count: 0,
        }
    }

    /// Record a measured dispatch latency in milliseconds and adapt the scale factor.
    pub fn record_dispatch(&mut self, dispatch_ms: u64) {
        if dispatch_ms == 0 {
            return;
        }
        self.sample_count += 1;
        let d = dispatch_ms as f64;
        self.ema_latency_ms =
            if self.sample_count <= 1 { d } else { 0.8 * self.ema_latency_ms + 0.2 * d };

        if dispatch_ms < 50 {
            self.scale = (self.scale + 0.10).min(4.0);
        } else if dispatch_ms > 150 {
            self.scale = (self.scale * 0.80).max(0.2);
        }
    }
}

/// DirectML hardware governor for Windows DirectX 12 Compute.
#[cfg(target_os = "windows")]
pub struct DirectMlGovernor {
    total_memory_bytes: usize,
    current_usage_bytes: AtomicUsize,
    last_usage_refresh: Mutex<Instant>,
    aimd: Mutex<AimdController>,
}

#[cfg(target_os = "windows")]
impl DirectMlGovernor {
    /// Create a new DirectML governor with auto-detected or overridden VRAM.
    pub fn new() -> Self {
        let vram_mb = if let Ok(val) = std::env::var("CTX_VRAM_MB") {
            val.parse::<usize>().unwrap_or_else(|_| detect_gpu_vram_mb())
        } else {
            detect_gpu_vram_mb()
        };
        let total_memory_bytes = vram_mb.max(256) * 1024 * 1024;
        let governor = Self {
            total_memory_bytes,
            current_usage_bytes: AtomicUsize::new(0),
            last_usage_refresh: Mutex::new(Instant::now() - Duration::from_secs(10)),
            aimd: Mutex::new(AimdController::new(100)),
        };
        governor.refresh_vram_usage_if_needed();
        governor
    }

    /// Construct a DirectMlGovernor with explicit VRAM in bytes (useful for testing).
    pub fn with_vram_bytes(vram_bytes: usize) -> Self {
        Self {
            total_memory_bytes: vram_bytes,
            current_usage_bytes: AtomicUsize::new(0),
            last_usage_refresh: Mutex::new(Instant::now() - Duration::from_secs(10)),
            aimd: Mutex::new(AimdController::new(100)),
        }
    }

    fn refresh_vram_usage_if_needed(&self) {
        if let Ok(mut last) = self.last_usage_refresh.try_lock() {
            if last.elapsed() >= Duration::from_secs(2) {
                *last = Instant::now();
                if let Ok(output) = std::process::Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-Command",
                        "Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPULocalAdapterMemory | Select-Object -ExpandProperty LocalUsage | Measure-Object -Maximum | Select-Object -ExpandProperty Maximum",
                    ])
                    .output()
                {
                    if output.status.success() {
                        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if let Ok(bytes) = text.parse::<usize>() {
                            self.current_usage_bytes.store(bytes, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Default for DirectMlGovernor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl HardwareGovernor for DirectMlGovernor {
    fn available_memory_bytes(&self) -> usize {
        self.refresh_vram_usage_if_needed();
        let usage = self.current_usage_bytes.load(Ordering::Relaxed);
        self.total_memory_bytes.saturating_sub(usage).max(256 * 1024 * 1024)
    }

    fn total_memory_bytes(&self) -> usize {
        self.total_memory_bytes
    }

    fn compute_adaptive_batch(&self, seq_len: usize, last_dispatch_ms: u64) -> usize {
        let mut aimd = self.aimd.lock().unwrap();
        if last_dispatch_ms > 0 {
            aimd.record_dispatch(last_dispatch_ms);
        }
        let scale = aimd.scale;
        drop(aimd);

        let available = self.available_memory_bytes();
        let activation_budget = (available as f64 * 0.70) as usize;

        let seq_len = seq_len.max(1);
        let per_chunk_attention_bytes = (3 * 12 * seq_len * seq_len * 4).max(2048);
        let batch_by_mem = activation_budget / per_chunk_attention_bytes;

        let max_tokens = (activation_budget / 32).clamp(2_048, 8_192);
        let batch_by_tokens = max_tokens / seq_len;

        let tdr_safe_cap = match seq_len {
            s if s > 768 => 8,
            s if s > 384 => 16,
            s if s > 128 => 32,
            _ => 64,
        };

        let base_batch = batch_by_mem.min(batch_by_tokens).min(tdr_safe_cap).max(1);
        let scaled = ((base_batch as f64) * scale).round() as usize;
        scaled.clamp(1, tdr_safe_cap)
    }

    fn target_slice_ms(&self) -> u64 {
        100
    }

    fn provider_name(&self) -> &str {
        "DirectML"
    }
}

/// CoreML / Metal hardware governor for macOS Apple Silicon.
#[cfg(target_os = "macos")]
pub struct CoreMlGovernor {
    total_memory_bytes: usize,
    aimd: Mutex<AimdController>,
}

#[cfg(target_os = "macos")]
impl CoreMlGovernor {
    /// Create a new CoreML governor with unified system memory.
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = sys.total_memory() as usize;
        Self {
            total_memory_bytes: if total == 0 { 16 * 1024 * 1024 * 1024 } else { total },
            aimd: Mutex::new(AimdController::new(100)),
        }
    }
}

#[cfg(target_os = "macos")]
impl Default for CoreMlGovernor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
impl HardwareGovernor for CoreMlGovernor {
    fn available_memory_bytes(&self) -> usize {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let free = sys.available_memory() as usize;
        if free == 0 {
            8 * 1024 * 1024 * 1024
        } else {
            free
        }
    }

    fn total_memory_bytes(&self) -> usize {
        self.total_memory_bytes
    }

    fn compute_adaptive_batch(&self, seq_len: usize, last_dispatch_ms: u64) -> usize {
        let mut aimd = self.aimd.lock().unwrap();
        if last_dispatch_ms > 0 {
            aimd.record_dispatch(last_dispatch_ms);
        }
        let scale = aimd.scale;
        drop(aimd);

        let available = self.available_memory_bytes();
        let activation_budget = (available as f64 * 0.70) as usize;

        let seq_len = seq_len.max(1);
        let per_chunk_attention_bytes = (3 * 12 * seq_len * seq_len * 4).max(2048);
        let batch_by_mem = activation_budget / per_chunk_attention_bytes;
        let max_tokens = (activation_budget / 32).clamp(8_192, 65_536);
        let batch_by_tokens = max_tokens / seq_len;

        let max_cap = match seq_len {
            s if s > 768 => 32,
            s if s > 384 => 96,
            s if s > 128 => 192,
            _ => 256,
        };

        let base_batch = batch_by_mem.min(batch_by_tokens).min(max_cap).max(1);
        let scaled = ((base_batch as f64) * scale).round() as usize;
        scaled.clamp(1, max_cap)
    }

    fn target_slice_ms(&self) -> u64 {
        100
    }

    fn provider_name(&self) -> &str {
        "CoreML"
    }
}

/// CPU hardware governor for Linux, Docker, or fallback environments.
pub struct CpuGovernor {
    total_memory_bytes: usize,
    aimd: Mutex<AimdController>,
}

impl CpuGovernor {
    /// Create a new CPU governor using host physical memory introspection.
    pub fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = sys.total_memory() as usize;
        Self {
            total_memory_bytes: if total == 0 { 8 * 1024 * 1024 * 1024 } else { total },
            aimd: Mutex::new(AimdController::new(100)),
        }
    }
}

impl Default for CpuGovernor {
    fn default() -> Self {
        Self::new()
    }
}

impl HardwareGovernor for CpuGovernor {
    fn available_memory_bytes(&self) -> usize {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let free = sys.available_memory() as usize;
        if free == 0 {
            4 * 1024 * 1024 * 1024
        } else {
            free
        }
    }

    fn total_memory_bytes(&self) -> usize {
        self.total_memory_bytes
    }

    fn compute_adaptive_batch(&self, seq_len: usize, last_dispatch_ms: u64) -> usize {
        let mut aimd = self.aimd.lock().unwrap();
        if last_dispatch_ms > 0 {
            aimd.record_dispatch(last_dispatch_ms);
        }
        drop(aimd);

        let l3_cache_budget = 64 * 1024 * 1024;
        let per_chunk_bytes = (3 * 12 * seq_len.max(1) * seq_len.max(1) * 4).max(2048);
        let batch_by_cache = l3_cache_budget / per_chunk_bytes;
        let cpu_cap = match seq_len {
            s if s > 512 => 8,
            s if s > 256 => 12,
            _ => 16,
        };
        batch_by_cache.min(cpu_cap).max(1)
    }

    fn target_slice_ms(&self) -> u64 {
        100
    }

    fn provider_name(&self) -> &str {
        "CPU"
    }
}

/// Get the default hardware governor for the current execution platform.
pub fn default_hardware_governor() -> Arc<dyn HardwareGovernor> {
    #[cfg(target_os = "windows")]
    {
        Arc::new(DirectMlGovernor::new())
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(CoreMlGovernor::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Arc::new(CpuGovernor::new())
    }
}
