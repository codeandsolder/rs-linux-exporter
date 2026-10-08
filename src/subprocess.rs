use std::io::Read;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use wait_timeout::ChildExt;

pub const DEFAULT_MAX_OUTPUT_BYTES: u64 = 1 << 20;

fn read_child_output(
    mut reader: impl Read,
    max_output_bytes: u64,
    description: &str,
) -> Result<Vec<u8>, String> {
    let mut limited = (&mut reader).take(max_output_bytes.saturating_add(1));
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {description} output: {error}"))?;
    if bytes.len() as u64 > max_output_bytes {
        return Err(format!(
            "{description} output exceeded {max_output_bytes} bytes"
        ));
    }
    Ok(bytes)
}

fn spawn_output_reader<R>(
    reader: R,
    max_output_bytes: u64,
    description: String,
) -> JoinHandle<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || read_child_output(reader, max_output_bytes, &description))
}

fn join_output_reader(
    handle: JoinHandle<Result<Vec<u8>, String>>,
    description: &str,
) -> Result<Vec<u8>, String> {
    handle
        .join()
        .map_err(|_| format!("{description} output reader thread panicked"))?
}

fn run_bounded_impl(
    mut command: Command,
    timeout: Duration,
    max_output_bytes: u64,
    description: &str,
    require_success: bool,
) -> Result<String, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("start {description}: {error}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{description} stdout pipe was not created"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{description} stderr pipe was not created"))?;
    let stdout_reader =
        spawn_output_reader(stdout, max_output_bytes, format!("{description} stdout"));
    let stderr_reader =
        spawn_output_reader(stderr, max_output_bytes, format!("{description} stderr"));

    let status = child
        .wait_timeout(timeout)
        .map_err(|error| format!("wait for {description}: {error}"))?;
    let timed_out = status.is_none();
    if timed_out {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| format!("reap {description}: {error}"))?;
    let stdout = join_output_reader(stdout_reader, description)?;
    let stderr = join_output_reader(stderr_reader, description)?;

    if timed_out {
        return Err(format!(
            "{description} timed out after {} ms",
            timeout.as_millis()
        ));
    }
    if require_success && !status.success() {
        return Err(format!(
            "{description} exited with {status}: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    String::from_utf8(stdout)
        .map_err(|error| format!("{description} output was not UTF-8: {error}"))
}

pub fn run_bounded(
    command: Command,
    timeout: Duration,
    max_output_bytes: u64,
    description: &str,
) -> Result<String, String> {
    run_bounded_impl(command, timeout, max_output_bytes, description, true)
}

pub fn run_bounded_allow_failure(
    command: Command,
    timeout: Duration,
    max_output_bytes: u64,
    description: &str,
) -> Result<String, String> {
    run_bounded_impl(command, timeout, max_output_bytes, description, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_mode_returns_stdout_from_nonzero_exit() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf useful; exit 2"]);
        let output =
            run_bounded_allow_failure(command, Duration::from_secs(1), 1024, "nonzero command")
                .expect("nonzero status should be permitted");
        assert_eq!(output, "useful");
    }

    #[test]
    fn returns_stdout() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf hello"]);
        let output = run_bounded(command, Duration::from_secs(1), 1024, "test command")
            .expect("command should succeed");
        assert_eq!(output, "hello");
    }

    #[test]
    fn enforces_timeout() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 1"]);
        let error = run_bounded(command, Duration::from_millis(10), 1024, "slow command")
            .expect_err("command should time out");
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn enforces_output_limit_without_pipe_deadlock() {
        let mut command = Command::new("sh");
        command.args(["-c", "yes x | head -c 4096"]);
        let error = run_bounded(command, Duration::from_secs(1), 128, "large command")
            .expect_err("command should exceed output cap");
        assert!(error.contains("exceeded 128 bytes"), "{error}");
    }
}
