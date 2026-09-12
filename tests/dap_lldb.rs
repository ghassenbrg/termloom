//! End-to-end validation against a real debug adapter.
//!
//! Uses `lldb-dap` (shipped with Xcode/LLVM) to debug a tiny C program:
//! launch, hit a breakpoint, inspect the stack and variables, step, continue
//! and terminate. Skips itself when no adapter is installed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use termloom::domain::debug::DebugStatus;
use termloom::services::dap::{
    DebugCommand, DebugEvent, DebugLaunchConfig, DebugSession, DebugSink,
};

/// Locate `lldb-dap`, including inside the active Xcode toolchain.
fn find_adapter() -> Option<String> {
    if let Some(paths) = std::env::var_os("PATH") {
        if let Some(found) = std::env::split_paths(&paths)
            .map(|dir| dir.join("lldb-dap"))
            .find(|path| path.is_file())
        {
            return Some(found.to_string_lossy().to_string());
        }
    }
    let output = Command::new("xcrun")
        .args(["-f", "lldb-dap"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty() && Path::new(&path).is_file()).then_some(path)
}

/// Compile a small program with a known breakpoint line.
fn build_program(dir: &Path) -> Option<PathBuf> {
    let source = dir.join("main.c");
    std::fs::write(
        &source,
        r#"#include <stdio.h>

int add(int a, int b) {
    int sum = a + b;
    return sum;
}

int main(void) {
    int total = add(2, 40);
    printf("total=%d\n", total);
    return 0;
}
"#,
    )
    .ok()?;
    let binary = dir.join("program");
    let status = Command::new("cc")
        .args(["-g", "-O0"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .status()
        .ok()?;
    status.success().then_some(binary)
}

struct Events {
    rx: mpsc::Receiver<DebugEvent>,
    seen: Vec<DebugEvent>,
}

impl Events {
    fn wait<T>(&mut self, mut predicate: impl FnMut(&DebugEvent) -> Option<T>) -> Option<T> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            match self.rx.recv_timeout(Duration::from_secs(5)) {
                Ok(event) => {
                    let matched = predicate(&event);
                    self.seen.push(event);
                    if let Some(value) = matched {
                        return Some(value);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return None,
            }
        }
        None
    }
}

#[test]
fn debugs_a_real_program_end_to_end() {
    let Some(adapter) = find_adapter() else {
        eprintln!("skipping: no lldb-dap adapter found");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some(binary) = build_program(dir.path()) else {
        eprintln!("skipping: no C compiler available");
        return;
    };
    let source = dir.path().join("main.c");

    // Break on `int sum = a + b;` (0-based line 3).
    let mut breakpoints = HashMap::new();
    breakpoints.insert(source.clone(), vec![3]);

    let config = DebugLaunchConfig {
        name: "probe".into(),
        type_name: "lldb".into(),
        attach: false,
        adapter_command: adapter,
        adapter_args: Vec::new(),
        adapter_env: Vec::new(),
        cwd: dir.path().to_path_buf(),
        arguments: serde_json::json!({
            "program": binary.to_string_lossy(),
            "stopOnEntry": false
        }),
        breakpoints,
    };

    let (tx, rx) = mpsc::channel();
    let sink: DebugSink = Arc::new(move |event| {
        let _ = tx.send(event);
    });
    let session = DebugSession::start(config, sink).expect("adapter should start");
    let mut events = Events {
        rx,
        seen: Vec::new(),
    };

    // 1. Initialize and capability negotiation.
    let capabilities = events
        .wait(|event| match event {
            DebugEvent::Initialized { capabilities, .. } => Some(*capabilities),
            _ => None,
        })
        .expect("adapter should initialize");
    assert!(
        capabilities.configuration_done,
        "lldb-dap supports configurationDone"
    );
    assert_eq!(DebugStatus::Starting.label(), "starting");

    // 2. Breakpoint verified by the adapter.
    let verified = events
        .wait(|event| match event {
            DebugEvent::BreakpointsVerified { lines, .. } if !lines.is_empty() => {
                Some(lines.clone())
            }
            _ => None,
        })
        .expect("the breakpoint should be verified");
    assert_eq!(verified, vec![3]);

    // 3. The program stops at the breakpoint with a usable call stack.
    let frames = events
        .wait(|event| match event {
            DebugEvent::Stopped { frames, .. } if !frames.is_empty() => Some(frames.clone()),
            _ => None,
        })
        .expect("the program should stop at the breakpoint");
    assert_eq!(frames[0].name.contains("add"), true, "{:?}", frames[0]);
    assert_eq!(frames[0].line, 4, "DAP reports 1-based lines");
    assert!(
        frames.iter().any(|frame| frame.name.contains("main")),
        "the stack should include main: {frames:?}"
    );

    // 4. Variables of the stopped frame.
    let variables = events
        .wait(|event| match event {
            DebugEvent::Variables(variables) if variables.len() > 1 => Some(variables.clone()),
            _ => None,
        })
        .expect("variables should be reported");
    let names: Vec<&str> = variables.iter().map(|v| v.name.as_str()).collect();
    assert!(names.contains(&"a"), "{names:?}");
    assert!(names.contains(&"b"), "{names:?}");
    let b = variables.iter().find(|v| v.name == "b").unwrap();
    assert_eq!(b.value, "40");

    // 5. Stepping keeps the session alive and produces a new stop.
    session.send(DebugCommand::StepOver).unwrap();
    let stepped = events
        .wait(|event| match event {
            DebugEvent::Stopped { reason, frames, .. } if !frames.is_empty() => {
                Some((reason.clone(), frames[0].line))
            }
            _ => None,
        })
        .expect("stepping should stop again");
    assert!(stepped.1 >= 4, "stepped to line {}", stepped.1);

    // 6. Continue to completion and terminate.
    session.send(DebugCommand::Continue).unwrap();
    let output = events.wait(|event| match event {
        DebugEvent::Output { text, .. } if text.contains("total=42") => Some(text.clone()),
        DebugEvent::Terminated => Some("terminated".into()),
        DebugEvent::Exited { .. } => Some("exited".into()),
        _ => None,
    });
    assert!(
        output.is_some(),
        "the program should finish: {:?}",
        events.seen
    );

    session.send(DebugCommand::Stop).ok();
}

#[test]
fn a_missing_adapter_is_reported_without_panicking() {
    let config = DebugLaunchConfig {
        name: "broken".into(),
        type_name: "none".into(),
        attach: false,
        adapter_command: "definitely-not-a-debug-adapter".into(),
        adapter_args: Vec::new(),
        adapter_env: Vec::new(),
        cwd: std::env::temp_dir(),
        arguments: serde_json::json!({}),
        breakpoints: HashMap::new(),
    };
    let sink: DebugSink = Arc::new(|_| {});
    assert!(DebugSession::start(config, sink).is_err());
}
