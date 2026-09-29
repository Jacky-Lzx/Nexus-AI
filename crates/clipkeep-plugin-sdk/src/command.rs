use crate::SearchContext;
use std::{
    io::{self, Read},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// Drain both pipes while waiting, and kill/reap the child on cancellation or excess output.
pub fn command_output(
    command: &mut Command,
    context: &SearchContext,
    max_bytes: usize,
) -> io::Result<Output> {
    context.check().map_err(io::Error::other)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let exceeded = Arc::new(AtomicBool::new(false));
    fn drain(
        mut reader: impl Read + Send + 'static,
        limit: usize,
        exceeded: Arc<AtomicBool>,
    ) -> thread::JoinHandle<io::Result<Vec<u8>>> {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0; 8192];
            loop {
                let count = reader.read(&mut chunk)?;
                if count == 0 {
                    return Ok(bytes);
                }
                let keep = count.min(limit.saturating_sub(bytes.len()));
                bytes.extend_from_slice(&chunk[..keep]);
                if keep < count {
                    exceeded.store(true, Ordering::Relaxed);
                }
            }
        })
    }
    let stdout = drain(child.stdout.take().unwrap(), max_bytes, exceeded.clone());
    let stderr = drain(child.stderr.take().unwrap(), max_bytes, exceeded.clone());
    let result = loop {
        if context.is_cancelled() || exceeded.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            break Err(io::Error::other(if context.is_cancelled() {
                "search cancelled"
            } else {
                "command output limit exceeded"
            }));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(error);
            }
        }
    };
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    if exceeded.load(Ordering::Relaxed) {
        return Err(io::Error::other("command output limit exceeded"));
    }
    context.check().map_err(io::Error::other)?;
    Ok(Output {
        status: result?,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancels_and_reaps_a_slow_child() {
        let context = SearchContext::new(1, Some(30));
        let start = std::time::Instant::now();
        assert!(command_output(Command::new("/bin/sleep").arg("10"), &context, 1024).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn bounds_output_without_pipe_deadlock() {
        assert!(
            command_output(
                &mut Command::new("/usr/bin/yes"),
                &SearchContext::new(1, Some(2000)),
                1024
            )
            .is_err()
        );
    }
}
