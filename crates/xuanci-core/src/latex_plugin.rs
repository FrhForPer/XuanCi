//! Optional external LaTeX-to-text renderer.
//!
//! The plugin is a separate executable, installed by the user. The host sends
//! one JSON request on stdin and receives styled text segments on stdout. No
//! images or temporary files are involved.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const PLUGIN_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SEGMENT_TEXT: usize = 20_000;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathStyle {
    Normal,
    Super,
    Sub,
}

#[derive(Debug, Clone)]
pub struct MathSegment {
    pub text: String,
    pub style: MathStyle,
}

#[derive(Debug, Clone)]
pub struct MathFragment {
    pub segments: Vec<MathSegment>,
}

/// Returns the plugin version when the executable speaks our protocol.
pub fn version(exe: &Path) -> Option<String> {
    let mut command = Command::new(exe);
    command.arg("--version");
    configure_stdio(&mut command, false);
    let output = run_with_timeout(command, None, Duration::from_secs(5))?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.contains("xuanci-latex") {
        Some(text)
    } else {
        None
    }
}

pub fn probe(exe: &Path) -> bool {
    exe.is_file() && version(exe).is_some()
}

/// Converts one formula through the external plugin. Returns None on any error
/// so the caller can fall back to showing the LaTeX source.
pub fn render_formula(exe: &Path, tex: &str, display: bool) -> Option<MathFragment> {
    if !exe.is_file() || tex.trim().is_empty() || tex.len() > 20_000 {
        return None;
    }

    let request = json!({ "tex": tex, "display": display });
    let payload = serde_json::to_vec(&request).ok()?;

    let mut command = Command::new(exe);
    configure_stdio(&mut command, true);
    let output = run_with_timeout(command, Some(payload), PLUGIN_TIMEOUT)?;
    parse_output(&output)
}

fn configure_stdio(command: &mut Command, with_stdin: bool) {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if with_stdin {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn run_with_timeout(
    mut command: Command,
    payload: Option<Vec<u8>>,
    timeout: Duration,
) -> Option<Output> {
    let mut child = command.spawn().ok()?;

    let writer = if let Some(payload) = payload {
        let mut stdin = child.stdin.take()?;
        Some(std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
            let _ = stdin.flush();
        }))
    } else {
        None
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Drain pipes while waiting. Otherwise a chatty plugin can fill a pipe
    // buffer and block before exiting while the parent is polling try_wait.
    let out_thread = stdout.map(|mut reader| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = reader.read_to_end(&mut buffer);
            buffer
        })
    });
    let err_thread = stderr.map(|mut reader| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = reader.read_to_end(&mut buffer);
            buffer
        })
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                break child.wait().ok();
            }
        }
    };

    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let stdout = out_thread
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = err_thread
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    status.map(|status| Output {
        status,
        stdout,
        stderr,
    })
}

fn parse_output(output: &Output) -> Option<MathFragment> {
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))?;
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return None;
    }

    let raw_segments = value.get("segments").and_then(Value::as_array)?;
    let mut segments = Vec::new();
    let mut total = 0usize;
    for raw in raw_segments.iter().take(4000) {
        let Some(text) = raw.get("text").and_then(Value::as_str) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        total += text.len();
        if total > MAX_SEGMENT_TEXT {
            return None;
        }
        let style = match raw.get("style").and_then(Value::as_str) {
            Some("super") => MathStyle::Super,
            Some("sub") => MathStyle::Sub,
            _ => MathStyle::Normal,
        };
        segments.push(MathSegment {
            text: text.to_string(),
            style,
        });
    }
    if segments.is_empty() {
        return None;
    }
    Some(MathFragment { segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_plugin() {
        let path = std::env::temp_dir().join("xuanci-no-such-latex-plugin.exe");
        assert!(!probe(&path));
        assert!(render_formula(&path, "x^2", false).is_none());
    }
}
