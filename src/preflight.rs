//! Startup pre-flight checks that catch the three classic ways a waxum
//! instance drops WhatsApp connections in the field:
//!
//! 1. **FD exhaustion.** WhatsApp's underlying TCP + TLS keeps ~50-70 file
//!    descriptors per active session (WebSocket, media downloads, sqlite
//!    handles). A Docker container that inherits the default 1024 `nofile`
//!    soft limit wedges around session 14 and the newest connections start
//!    dropping. We inspect `RLIMIT_NOFILE` on Linux and log a warning when
//!    the soft limit is below the recommended floor of 65536.
//!
//! 2. **Two waxum instances sharing the same Signal Store.** If two
//!    processes open the same `WHATSAPP_STORAGE_PATH`, WhatsApp servers
//!    see the second connect and issue a `<failure reason='replaced'/>`
//!    against the first — the two instances fight, each disconnecting the
//!    other in a loop. We drop a pidfile in the storage root and refuse
//!    to boot if another *live* PID already holds it.
//!
//! 3. **Cold-start reconnect burst.** All previously-paired sessions try
//!    to reconnect in parallel on boot; if that spike hits WA servers too
//!    fast they'll rate-limit the whole IP for a few minutes. The
//!    `reconnect_all_on_startup` loop staggers spawns via
//!    [`session_startup_stagger`] (default 500 ms per session).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct InstanceLock {
    path: PathBuf,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Refuse to boot if another waxum process is already holding this storage
/// path. Returns an `InstanceLock` that removes the pidfile on drop.
///
/// The check is a pidfile at `{storage}/.waxum.lock` containing the owner
/// PID. A stale file (PID no longer running) is silently reclaimed, so a
/// hard-killed container that never got to clean up on the way down does
/// not brick the next start.
pub fn acquire_instance_lock(storage_path: &str) -> Result<InstanceLock, String> {
    let path = Path::new(storage_path).join(".waxum.lock");
    if let Err(e) = fs::create_dir_all(storage_path) {
        return Err(format!("cannot create storage dir {storage_path}: {e}"));
    }

    if let Ok(existing) = fs::read_to_string(&path) {
        let pid: Option<u32> = existing.trim().parse().ok();
        if let Some(pid) = pid {
            if pid_is_alive(pid) && pid != std::process::id() {
                return Err(format!(
                    "another waxum process (pid {pid}) already owns {}. \
                     Two instances sharing the same WHATSAPP_STORAGE_PATH \
                     causes WhatsApp servers to disconnect both in a loop. \
                     Stop the other instance, or point this one at a \
                     different WHATSAPP_STORAGE_PATH.",
                    path.display()
                ));
            }
        }
    }

    fs::write(&path, std::process::id().to_string())
        .map_err(|e| format!("cannot write pidfile {}: {e}", path.display()))?;
    Ok(InstanceLock { path })
}

#[cfg(target_os = "linux")]
fn pid_is_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "windows")]
fn pid_is_alive(pid: u32) -> bool {
    use std::process::Command;
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn pid_is_alive(_pid: u32) -> bool {
    true
}

/// Warn (do not abort) when `RLIMIT_NOFILE` is below the recommended floor
/// for a WhatsApp gateway. Linux only; other OSes are a no-op.
pub fn check_fd_limit() {
    #[cfg(target_os = "linux")]
    {
        const RECOMMENDED: u64 = 65536;
        let soft = read_proc_limit("Max open files");
        if let Some(soft) = soft {
            if soft < RECOMMENDED {
                tracing::warn!(
                    soft_limit = soft,
                    recommended = RECOMMENDED,
                    "RLIMIT_NOFILE below recommended floor. Each active \
                     WhatsApp session holds ~50-70 fds; with the current \
                     limit the process will wedge around {} sessions and \
                     new connections will start dropping. In docker-compose, \
                     add `ulimits: nofile: {{soft: 65536, hard: 65536}}`.",
                    soft / 70
                );
            } else {
                tracing::info!(soft_limit = soft, "FD limit OK");
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::debug!("FD limit check skipped (non-Linux)");
    }
}

#[cfg(target_os = "linux")]
fn read_proc_limit(key: &str) -> Option<u64> {
    let text = fs::read_to_string("/proc/self/limits").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            let cols: Vec<&str> = rest.split_whitespace().collect();
            if let Some(first) = cols.first() {
                return first.parse().ok();
            }
        }
    }
    None
}

/// Delay between spawning each auto-reconnect on startup. Configurable via
/// `SESSION_STARTUP_STAGGER_MS`. Defaults to 500 ms — high enough that a
/// 500-session cold start looks like a slow ramp to WhatsApp servers
/// instead of a flood that trips their per-IP rate limiter.
pub fn session_startup_stagger() -> Duration {
    let ms = std::env::var("SESSION_STARTUP_STAGGER_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(500);
    Duration::from_millis(ms)
}
