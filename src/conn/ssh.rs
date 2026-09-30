use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use super::SshSpec;

const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// A `ssh -N -L` port forward; the ssh process is killed when this is dropped.
pub struct Tunnel {
    child: Child,
    pub local_port: u16,
}

fn free_local_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn forward_spec(local_port: u16, remote_host: &str, remote_port: u16) -> String {
    let host = if remote_host.contains(':') { format!("[{remote_host}]") } else { remote_host.to_string() };
    format!("127.0.0.1:{local_port}:{host}:{remote_port}")
}

fn ssh_args(ssh: &SshSpec, forward: &str) -> Vec<String> {
    let mut args: Vec<String> =
        ["-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30", "-L", forward].map(String::from).into();
    if let Some(p) = ssh.port {
        args.extend(["-p".into(), p.to_string()]);
    }
    if let Some(i) = &ssh.identity {
        args.extend(["-i".into(), i.display().to_string()]);
    }
    args.push(match &ssh.user {
        Some(u) => format!("{u}@{}", ssh.host),
        None => ssh.host.clone(),
    });
    args
}

impl Tunnel {
    pub async fn open(ssh: &SshSpec, remote_host: &str, remote_port: u16) -> anyhow::Result<Tunnel> {
        let local_port = free_local_port().context("no free local port for the SSH tunnel")?;
        let mut child = Command::new("ssh")
            .args(ssh_args(ssh, &forward_spec(local_port, remote_host, remote_port)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("cannot run `ssh` (is OpenSSH installed?)")?;

        // Drain stderr continuously so a chatty ssh never blocks, and keep it for error messages.
        let stderr = Arc::new(Mutex::new(String::new()));
        let reader = child.stderr.take().map(|mut pipe| {
            let buf = stderr.clone();
            tokio::spawn(async move {
                let mut chunk = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut chunk).await {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut b) = buf.lock() {
                        b.push_str(&String::from_utf8_lossy(&chunk[..n]));
                    }
                }
            })
        });
        let stderr_text = |stderr: &Arc<Mutex<String>>| stderr.lock().map(|s| s.trim().to_string()).unwrap_or_default();

        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                if let Some(r) = reader {
                    let _ = tokio::time::timeout(Duration::from_secs(1), r).await;
                }
                bail!("ssh tunnel to {} failed ({status}): {}", ssh.host, stderr_text(&stderr));
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
                return Ok(Tunnel { child, local_port });
            }
            if started.elapsed() > OPEN_TIMEOUT {
                let _ = child.start_kill();
                bail!(
                    "ssh tunnel to {} did not come up within {}s: {}",
                    ssh.host,
                    OPEN_TIMEOUT.as_secs(),
                    stderr_text(&stderr)
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn builds_ssh_command_line() {
        let spec = SshSpec { user: Some("deploy".into()), host: "bastion".into(), port: Some(2222), identity: Some(PathBuf::from("/k")) };
        let args = ssh_args(&spec, &forward_spec(4000, "db.internal", 5432));
        assert_eq!(
            args,
            [
                "-N", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30", "-L",
                "127.0.0.1:4000:db.internal:5432", "-p", "2222", "-i", "/k", "deploy@bastion"
            ]
        );
        assert_eq!(forward_spec(1, "::1", 3306), "127.0.0.1:1:[::1]:3306");
    }

    #[tokio::test]
    async fn failed_tunnel_surfaces_ssh_stderr() {
        if which::which("ssh").is_err() {
            println!("SKIP: ssh not installed");
            return;
        }
        let spec = SshSpec { user: None, host: "quarry-no-such-host.invalid".into(), port: None, identity: None };
        let err = Tunnel::open(&spec, "localhost", 5432).await.err().expect("tunnel must fail").to_string();
        assert!(err.starts_with("ssh tunnel to quarry-no-such-host.invalid"), "{err}");
        assert!(err.len() > "ssh tunnel to quarry-no-such-host.invalid failed (): ".len(), "stderr included: {err}");
    }
}
