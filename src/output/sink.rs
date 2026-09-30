use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Extra destinations for results: `tee` (every result), `\o`/`\once` (next result only) and
/// `\|` (next result piped to a shell command). Always written without colors.
#[derive(Default)]
pub struct Sinks {
    tee: Option<(PathBuf, File)>,
    once: Option<File>,
    pipe: Option<String>,
}

pub(crate) fn open_private(path: &Path, overwrite: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).write(true);
    if overwrite {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

impl Sinks {
    pub fn tee(&mut self, path: impl AsRef<Path>, overwrite: bool) -> io::Result<()> {
        let path = path.as_ref();
        let file = open_private(path, overwrite)?;
        self.tee = Some((path.to_path_buf(), file));
        Ok(())
    }

    pub fn notee(&mut self) {
        self.tee = None;
    }

    pub fn tee_path(&self) -> Option<&Path> {
        self.tee.as_ref().map(|(p, _)| p.as_path())
    }

    /// The next result goes to `path` (and not to the screen).
    pub fn once(&mut self, path: impl AsRef<Path>, overwrite: bool) -> io::Result<()> {
        self.once = Some(open_private(path.as_ref(), overwrite)?);
        Ok(())
    }

    /// The next result is piped to `command` (run with `sh -c`) instead of the screen.
    pub fn pipe_once(&mut self, command: impl Into<String>) {
        self.pipe = Some(command.into());
    }

    /// True when the next result is redirected, i.e. the caller should not print it to the screen.
    pub fn is_redirected_once(&self) -> bool {
        self.once.is_some() || self.pipe.is_some()
    }

    /// True when `write_result` would write anywhere (tee file, once file or pending pipe).
    pub fn is_active(&self) -> bool {
        self.tee.is_some() || self.is_redirected_once()
    }

    /// Writes one result to the tee file and consumes a pending once / pipe target.
    pub fn write_result(&mut self, plain_text: &str) -> io::Result<()> {
        let mut result = Ok(());
        if let Some((_, f)) = &mut self.tee {
            result = f.write_all(plain_text.as_bytes()).and_then(|_| f.flush());
        }
        if let Some(mut f) = self.once.take() {
            let r = f.write_all(plain_text.as_bytes()).and_then(|_| f.flush());
            result = result.and(r);
        }
        if let Some(cmd) = self.pipe.take() {
            result = result.and(pipe_to(&cmd, plain_text));
        }
        result
    }
}

fn pipe_to(cmd: &str, text: &str) -> io::Result<()> {
    let mut child = Command::new("sh").arg("-c").arg(cmd).stdin(Stdio::piped()).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(text.as_bytes()) {
            Err(e) if e.kind() != io::ErrorKind::BrokenPipe => return Err(e),
            _ => {}
        }
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("`{cmd}` exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quarry-sink-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        dir
    }

    #[test]
    fn once_consumed_after_one_result_tee_persists() {
        let (tee, once) = (tmp("tee"), tmp("once"));
        let mut s = Sinks::default();
        s.tee(&tee, true).unwrap();
        s.once(&once, true).unwrap();
        assert!(s.is_redirected_once());
        s.write_result("a\n").unwrap();
        assert!(!s.is_redirected_once());
        s.write_result("b\n").unwrap();
        assert_eq!(std::fs::read_to_string(&tee).unwrap(), "a\nb\n");
        assert_eq!(std::fs::read_to_string(&once).unwrap(), "a\n");
        s.notee();
        s.write_result("c\n").unwrap();
        assert_eq!(std::fs::read_to_string(&tee).unwrap(), "a\nb\n");
        let _ = (std::fs::remove_file(tee), std::fs::remove_file(once));
    }

    #[test]
    fn tee_appends_unless_overwrite() {
        let p = tmp("append");
        std::fs::write(&p, "old\n").unwrap();
        let mut s = Sinks::default();
        s.tee(&p, false).unwrap();
        s.write_result("new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "old\nnew\n");
        s.tee(&p, true).unwrap();
        s.write_result("x\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x\n");
        let _ = std::fs::remove_file(p);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("perm");
        let mut s = Sinks::default();
        s.once(&p, true).unwrap();
        s.write_result("secret\n").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn pipe_once_runs_command_with_text_on_stdin() {
        let p = tmp("pipe");
        let mut s = Sinks::default();
        s.pipe_once(format!("cat > '{}'", p.display()));
        s.write_result("piped\n").unwrap();
        assert!(!s.is_redirected_once());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "piped\n");
        let _ = std::fs::remove_file(p);
    }
}
