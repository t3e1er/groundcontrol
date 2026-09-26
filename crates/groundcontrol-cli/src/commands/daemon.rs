//! Daemon lifecycle command handlers (`groundcontrol daemon start|stop|status|sync|restart`).

use std::time::{Duration, Instant};

use anyhow::Context;
use sysinfo::{Pid, System};

use groundcontrol_common::config::{
    get_logs_cache_dir, read_daemon_pid, remove_daemon_pid, write_daemon_pid, DaemonPidInfo,
};

use super::serve::is_server_healthy;

/// Check if a process ID is currently alive on the host system.
pub fn is_pid_alive(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    system.process(Pid::from_u32(pid)).is_some()
}

/// Terminate a process by PID.
pub fn kill_pid(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    if let Some(process) = system.process(Pid::from_u32(pid)) {
        process.kill()
    } else {
        false
    }
}

/// Normalize a bind address or server URL into a valid client-connectable URL.
///
/// On Windows and many operating systems, connecting to `0.0.0.0` or `[::]` fails because
/// `0.0.0.0` is `INADDR_ANY` (valid only for binding a listening socket, not as a client destination).
/// This maps `0.0.0.0` -> `127.0.0.1` and `[::]` -> `[::1]`.
pub fn normalize_probe_url(bind_or_server: &str) -> String {
    let mut url = if bind_or_server.starts_with("http://") || bind_or_server.starts_with("https://")
    {
        bind_or_server.to_string()
    } else {
        format!("http://{}", bind_or_server)
    };
    url = url.replace("://0.0.0.0:", "://127.0.0.1:").replace("://[::]:", "://[::1]:");
    url
}

/// Start the background daemon process in detached mode.
pub async fn handle_daemon_start(
    sync: bool,
    bind: &str,
    idle_timeout: u64,
    watch: bool,
    require_auth: bool,
) -> anyhow::Result<()> {
    let probe_url = normalize_probe_url(bind);
    let server_url = format!("http://{}", bind);

    // 1. Check if daemon is already alive
    if is_server_healthy(&probe_url).await {
        if let Some(info) = read_daemon_pid() {
            println!(
                "[!] groundcontrol daemon is already running (PID {}, {})",
                info.pid, info.bind
            );
        } else {
            println!("[!] A groundcontrol server is already listening on {}", server_url);
        }
        return Ok(());
    }

    // Clean up stale PID file if present
    if let Some(info) = read_daemon_pid() {
        if !is_pid_alive(info.pid) {
            let _ = remove_daemon_pid();
        }
    }

    println!("[*] Starting groundcontrol background daemon on {}...", bind);

    let current_exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(current_exe);
    cmd.args(["server", "--bind", bind, "--daemon", "--log-level", "info", "--log-format", "json"]);

    if sync {
        cmd.arg("--sync");
    }
    if watch {
        cmd.arg("--watch");
    }
    if idle_timeout > 0 {
        cmd.arg(format!("--idle-timeout={}", idle_timeout));
    }
    if require_auth {
        cmd.arg("--require-auth");
    }

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    let log_dir = get_logs_cache_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    if let Ok(log_file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("groundcontrol-daemon.log"))
    {
        cmd.stderr(log_file);
    } else {
        cmd.stderr(std::process::Stdio::null());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB);
    }

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                const DETACHED_PROCESS: u32 = 0x00000008;
                cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
                cmd.spawn().context("failed to spawn groundcontrol daemon process")?
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("failed to spawn groundcontrol daemon process");
            }
        }
    };
    let child_pid = child.id();

    // Poll /health until server responds (generous deadline when startup sync is requested)
    let wait_secs = if sync { 30 } else { 8 };
    let deadline = Instant::now() + Duration::from_secs(wait_secs);
    let mut up = false;
    while Instant::now() < deadline {
        if is_server_healthy(&probe_url).await {
            up = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    if !up {
        anyhow::bail!(
            "groundcontrol background daemon failed to respond on {} within deadline",
            bind
        );
    }

    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let pid_info =
        DaemonPidInfo { pid: child_pid, bind: bind.to_string(), started_at, token: None };
    let _ = write_daemon_pid(&pid_info);

    let log_path = get_logs_cache_dir().join("groundcontrol-daemon.jsonl");

    println!("[+] groundcontrol daemon started successfully!");
    println!("    ├─ PID      : {}", child_pid);
    println!("    ├─ Endpoint : {}", server_url);
    println!(
        "    ├─ Mode     : {}",
        if sync { "sync (startup delta scan active)" } else { "no-sync" }
    );
    println!(
        "    ├─ Watching : {}",
        if watch { "enabled (real-time reindexing)" } else { "disabled" }
    );
    println!("    ├─ Timeout  : {} mins (0 = disabled)", idle_timeout);
    println!("    └─ Log File : {}", log_path.display());

    Ok(())
}

/// Stop the running background daemon gracefully.
pub async fn handle_daemon_stop(server_url: &str) -> anyhow::Result<()> {
    let pid_info = read_daemon_pid();
    let probe_url = normalize_probe_url(server_url);
    let is_alive = is_server_healthy(&probe_url).await;

    if !is_alive && pid_info.is_none() {
        println!("[-] groundcontrol daemon is not running.");
        return Ok(());
    }

    println!("[*] Stopping groundcontrol daemon...");

    // 1. Try graceful shutdown via HTTP POST /shutdown
    let shutdown_url = format!("{}/shutdown", probe_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build()?;
    let mut stopped_via_http = false;

    if let Ok(resp) = client.post(&shutdown_url).send().await {
        if resp.status().is_success() {
            stopped_via_http = true;
        }
    }

    // 2. Wait up to 3 seconds for process to exit
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !is_server_healthy(&probe_url).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 3. Fallback to process kill if PID is known and still alive
    if let Some(info) = pid_info {
        if is_pid_alive(info.pid) {
            if !stopped_via_http {
                println!("[*] HTTP shutdown unresponsive; terminating PID {}...", info.pid);
            }
            let _ = kill_pid(info.pid);
        }
    }

    let _ = remove_daemon_pid();
    if is_server_healthy(&probe_url).await {
        println!("[!] Warning: Server is still responding on {server_url}. You may need to terminate the process manually.");
    } else {
        println!("[+] groundcontrol daemon stopped successfully.");
    }
    Ok(())
}

/// Probe daemon liveness, uptime, PID, and active corpora.
pub async fn handle_daemon_status(server_url: &str) -> anyhow::Result<()> {
    let pid_info = read_daemon_pid();
    let probe_url = normalize_probe_url(server_url);
    let health_url = format!("{}/status", probe_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build()?;

    match client.get(&health_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let json: serde_json::Value = resp.json().await?;
            let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("healthy");
            let pid = json
                .get("pid")
                .and_then(|v| v.as_u64())
                .map(|p| p as u32)
                .or_else(|| pid_info.as_ref().map(|i| i.pid))
                .unwrap_or(0);
            let uptime_secs = json.get("uptime_seconds").and_then(|v| v.as_u64()).unwrap_or(0);
            let corpora_count = json.get("corpora_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let corpora = json.get("corpora").and_then(|v| v.as_array());

            let uptime_formatted = if uptime_secs >= 3600 {
                format!("{}h {}m", uptime_secs / 3600, (uptime_secs % 3600) / 60)
            } else if uptime_secs >= 60 {
                format!("{}m {}s", uptime_secs / 60, uptime_secs % 60)
            } else {
                format!("{}s", uptime_secs)
            };

            println!("● groundcontrol daemon: RUNNING");
            println!("  ├─ PID       : {}", pid);
            println!("  ├─ Endpoint  : {}", server_url);
            println!("  ├─ Status    : {}", status);
            println!("  ├─ Uptime    : {}", uptime_formatted);
            print!("  └─ Corpora   : {} loaded", corpora_count);
            if let Some(list) = corpora {
                let names: Vec<&str> = list.iter().filter_map(|v| v.as_str()).collect();
                if !names.is_empty() {
                    print!(" ({})", names.join(", "));
                }
            }
            println!();

            if let Some(indexing) = json.get("indexing_progress") {
                if let Some(stage) = indexing.get("stage").and_then(|v| v.as_str()) {
                    let processed =
                        indexing.get("processed_files").and_then(|v| v.as_u64()).unwrap_or(0);
                    let total = indexing.get("total_files").and_then(|v| v.as_u64()).unwrap_or(0);
                    let fps =
                        indexing.get("files_per_second").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    println!(
                        "  [i] In-flight sync: {} (files: {}/{}, throughput: {:.1} files/s)",
                        stage, processed, total, fps
                    );
                }
            }
        }
        _ => {
            println!("○ groundcontrol daemon: STOPPED (not running)");
            if let Some(info) = pid_info {
                if is_pid_alive(info.pid) {
                    println!("  [!] Warning: Orphaned process with PID {} detected. Run 'groundcontrol daemon stop' to clean up.", info.pid);
                }
            }
        }
    }

    Ok(())
}

/// Trigger an incremental delta scan inside the running daemon via HTTP POST /sync.
pub async fn handle_daemon_sync(server_url: &str, corpus: Option<String>) -> anyhow::Result<()> {
    let probe_url = normalize_probe_url(server_url);
    if !is_server_healthy(&probe_url).await {
        anyhow::bail!(
            "groundcontrol daemon is not running on {}.\n\
             Start the daemon with 'groundcontrol daemon start' or run local sync with 'groundcontrol corpus sync'.",
            server_url
        );
    }

    println!("[*] Requesting daemon synchronization on {}...", server_url);
    let sync_url = format!("{}/sync", probe_url.trim_end_matches('/'));
    let client = reqwest::Client::builder().timeout(Duration::from_secs(300)).build()?;

    let payload = serde_json::json!({
        "corpus": corpus
    });

    let resp = client
        .post(&sync_url)
        .json(&payload)
        .send()
        .await
        .context("failed to communicate with daemon sync endpoint")?;

    if !resp.status().is_success() {
        anyhow::bail!("daemon sync request failed with HTTP {}", resp.status());
    }

    let json: serde_json::Value = resp.json().await?;
    if let Some(corpora) = json.get("corpora").and_then(|v| v.as_object()) {
        println!("[+] Daemon synchronization complete:");
        for (name, stats) in corpora {
            let status = stats.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            if status == "ok" {
                let new_count = stats.get("new_files").and_then(|v| v.as_u64()).unwrap_or(0);
                let mod_count = stats.get("modified_files").and_then(|v| v.as_u64()).unwrap_or(0);
                let del_count = stats.get("deleted_files").and_then(|v| v.as_u64()).unwrap_or(0);
                println!(
                    "    ├─ {}: {} new, {} modified, {} deleted",
                    name, new_count, mod_count, del_count
                );
            } else {
                let err = stats.get("error").and_then(|v| v.as_str()).unwrap_or("error");
                println!("    ├─ {}: FAILED ({})", name, err);
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&json)?);
    }

    Ok(())
}

/// Restart the background daemon.
pub async fn handle_daemon_restart(
    sync: bool,
    bind: &str,
    idle_timeout: u64,
    watch: bool,
    require_auth: bool,
) -> anyhow::Result<()> {
    let server_url = format!("http://{}", bind);
    handle_daemon_stop(&server_url).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    handle_daemon_start(sync, bind, idle_timeout, watch, require_auth).await
}
