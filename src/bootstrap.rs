//! Process setup that runs before the async runtime starts, so the
//! container image needs nothing but the waxum binary: no shell, no
//! `gosu`, no `curl`.
//!
//! The image used to be `debian-slim` with an entrypoint script that
//! chowned the data volumes and dropped to an unprivileged user with
//! `gosu`, plus `curl` for the healthcheck. A scan of that image found
//! 252 CVEs, 162 of them from `gosu` alone (a Go 1.19 binary) and the
//! rest from base packages waxum never calls. Both jobs now live here,
//! and the image is distroless.
//!
//! - [`drop_root_privileges`] replaces the entrypoint script. It only acts
//!   when `WAXUM_RUN_AS=uid:gid` is set (the Dockerfile sets it) *and* the
//!   process is root, so running waxum as root outside the image is left
//!   exactly as it was. It chowns the session storage directory and the
//!   SQLite directory, then drops supplementary groups, gid and uid. It
//!   runs while the process is still single-threaded.
//! - [`healthcheck`] replaces `curl -f http://127.0.0.1:$PORT/health`,
//!   as `waxum --healthcheck`.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `GET /health` on the local port. `true` only for a `200` reply.
pub fn healthcheck() -> bool {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3451);
    healthcheck_port(port)
}

fn healthcheck_port(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_secs(4);
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    if stream
        .write_all(b"GET /health HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut head = [0u8; 32];
    let n = stream.read(&mut head).unwrap_or(0);
    is_ok_status_line(&head[..n])
}

fn is_ok_status_line(head: &[u8]) -> bool {
    let line = String::from_utf8_lossy(head);
    let mut parts = line.split_whitespace();
    matches!(
        (parts.next(), parts.next()),
        (Some(proto), Some("200")) if proto.starts_with("HTTP/")
    )
}

/// Resolves with the signal's name once the process is asked to stop.
///
/// In the image waxum is PID 1, and the kernel does not apply the default
/// "terminate" action to PID 1: without a handler `SIGTERM` is ignored, so
/// `docker stop` waited out its 10 s grace period and then sent `SIGKILL`
/// on every deploy. The server stops as soon as this resolves rather than
/// draining connections, because long-lived ones (SSE, `connect/wait`)
/// would hold a drain open until the same `SIGKILL`.
pub async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut term), Ok(mut int)) => tokio::select! {
                _ = term.recv() => "SIGTERM",
                _ = int.recv() => "SIGINT",
            },
            _ => std::future::pending().await,
        }
    }
    #[cfg(not(unix))]
    {
        match tokio::signal::ctrl_c().await {
            Ok(()) => "Ctrl-C",
            Err(_) => std::future::pending().await,
        }
    }
}

/// Parses `WAXUM_RUN_AS`, e.g. `1000:1000`. Root (`0`) is refused for
/// either id: the point is to stop being root.
fn parse_run_as(raw: &str) -> Option<(u32, u32)> {
    let (uid, gid) = raw.trim().split_once(':')?;
    let (uid, gid) = (uid.parse::<u32>().ok()?, gid.parse::<u32>().ok()?);
    (uid != 0 && gid != 0).then_some((uid, gid))
}

/// Directories waxum writes to, which a volume mounted from an older
/// (root-running) image may still own as root.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(
        std::env::var("WHATSAPP_STORAGE_PATH").unwrap_or_else(|_| "./whatsapp_sessions".into()),
    )];
    let sqlite_file = std::env::var("SQLITE_PATH").ok().or_else(|| {
        std::env::var("DATABASE_URL")
            .ok()
            .and_then(|url| url.strip_prefix("sqlite://").map(str::to_string))
    });
    if let Some(parent) = sqlite_file
        .as_deref()
        .and_then(|f| Path::new(f).parent())
        .filter(|p| !p.as_os_str().is_empty())
    {
        dirs.push(parent.to_path_buf());
    }
    dirs
}

/// See the module docs. A no-op unless `WAXUM_RUN_AS` is set and the
/// process is root. Exits the process if the drop itself fails: carrying
/// on as root would silently undo the image's non-root guarantee.
#[cfg(unix)]
pub fn drop_root_privileges() {
    let Ok(raw) = std::env::var("WAXUM_RUN_AS") else {
        return;
    };
    if unsafe { libc::geteuid() } != 0 {
        return;
    }
    let Some((uid, gid)) = parse_run_as(&raw) else {
        eprintln!("waxum: WAXUM_RUN_AS must be <uid>:<gid> with non-zero ids, got {raw:?}");
        std::process::exit(1);
    };

    for dir in data_dirs() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("waxum: could not create {}: {e}", dir.display());
            continue;
        }
        chown_tree(&dir, uid, gid);
    }

    let dropped = unsafe {
        libc::setgroups(0, std::ptr::null()) == 0
            && libc::setgid(gid) == 0
            && libc::setuid(uid) == 0
    };
    if !dropped || unsafe { libc::geteuid() } == 0 {
        eprintln!(
            "waxum: failed to drop root privileges to {uid}:{gid}: {}",
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
pub fn drop_root_privileges() {}

/// `chown -R`, without following symlinks. Failures are reported and
/// skipped: one unreadable entry must not stop the gateway from starting.
#[cfg(unix)]
fn chown_tree(root: &Path, uid: u32, gid: u32) {
    use std::os::unix::fs::{lchown, MetadataExt};

    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.uid() != uid || meta.gid() != gid {
            if let Err(e) = lchown(&path, Some(uid), Some(gid)) {
                eprintln!("waxum: could not chown {}: {e}", path.display());
            }
        }
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn run_as_needs_two_non_root_ids() {
        assert_eq!(parse_run_as("1000:1000"), Some((1000, 1000)));
        assert_eq!(parse_run_as(" 65532:65532 "), Some((65532, 65532)));
        for bad in [
            "",
            "1000",
            "waxum:waxum",
            "0:1000",
            "1000:0",
            "0:0",
            "1000:",
            "-1:5",
        ] {
            assert_eq!(parse_run_as(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_200_status_line_is_healthy() {
        assert!(is_ok_status_line(b"HTTP/1.1 200 OK\r\n"));
        assert!(is_ok_status_line(b"HTTP/1.0 200 OK\r\ncontent-length: 2"));
        for bad in [
            &b"HTTP/1.1 503 Service Unavailable\r\n"[..],
            b"HTTP/1.1 401 Unauthorized\r\n",
            b"200 OK",
            b"",
        ] {
            assert!(!is_ok_status_line(bad));
        }
    }

    fn serve_once(reply: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 256];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply);
            }
        });
        port
    }

    #[test]
    fn healthcheck_follows_the_servers_answer() {
        assert!(healthcheck_port(serve_once(
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nOK"
        )));
        assert!(!healthcheck_port(serve_once(
            b"HTTP/1.1 503 Service Unavailable\r\n\r\n"
        )));
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(!healthcheck_port(closed), "nothing listening is unhealthy");
    }

    #[cfg(unix)]
    #[test]
    fn chown_tree_to_the_current_owner_walks_without_errors() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("waxum-chown-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/f"), b"x").unwrap();
        let meta = std::fs::metadata(&dir).unwrap();
        chown_tree(&dir, meta.uid(), meta.gid());
        assert_eq!(
            std::fs::metadata(dir.join("a/b/f")).unwrap().uid(),
            meta.uid()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
