//! End-to-end smoke test: run the real binary inside a pseudo terminal and
//! drive it with keystrokes, the way a user would.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize};

/// A running TermLoom under our own pty, with its screen parsed by vt100.
struct Harness {
    _dir: tempfile::TempDir,
    _home: tempfile::TempDir,
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Harness {
    fn start() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        // Config/state must live outside the workspace, or they would show up
        // in the explorer and shift the rows this test navigates.
        let home = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "# demo\n").unwrap();

        let pty = portable_pty::native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 40,
                cols: 150,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_termloom"));
        command.arg(root.to_string_lossy().to_string());
        command.cwd(root);
        command.env("TERM", "xterm-256color");
        // Keep the app's own state out of the developer's real config.
        command.env("HOME", home.path().to_string_lossy().to_string());
        command.env(
            "XDG_CONFIG_HOME",
            home.path().join("config").to_string_lossy().to_string(),
        );
        command.env(
            "XDG_STATE_HOME",
            home.path().join("state").to_string_lossy().to_string(),
        );

        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(vt100::Parser::new(40, 150, 2000)));

        let sink = Arc::clone(&parser);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().process(&buffer[..n]);
            }
        });

        Harness {
            _dir: dir,
            _home: home,
            writer,
            parser,
            child,
        }
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    /// Wait until the screen contains `needle`.
    fn wait_for(&self, needle: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self.screen().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Wait until the screen no longer contains `needle`.
    fn wait_gone(&self, needle: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if !self.screen().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn wait_exit(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn opens_browses_runs_a_terminal_and_quits() {
    let mut app = Harness::start();

    // 1. The workbench comes up on a real repository.
    assert!(
        app.wait_for("TermLoom"),
        "workbench did not start:\n{}",
        app.screen()
    );
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());
    assert!(app.wait_for("README.md"), "{}", app.screen());

    // 2. Open a file from the explorer: move down to README.md and press Enter.
    app.send(b"\x1b[B"); // Down
    app.send(b"\r");
    assert!(
        app.wait_for("# demo"),
        "file did not open:\n{}",
        app.screen()
    );

    // 3. Start a real terminal with the command prefix (Ctrl+Space, then T).
    app.send(b"\x00"); // Ctrl+Space
    app.send(b"t");
    assert!(
        app.wait_for("Terminal 1"),
        "terminal pane missing:\n{}",
        app.screen()
    );

    // 4. The pane is interactive: type a command into the child shell.
    app.send(b"echo termloom-pty-works\r");
    assert!(
        app.wait_for("termloom-pty-works"),
        "shell did not run the command:\n{}",
        app.screen()
    );

    // 5. The command palette opens and filters.
    app.send(b"\x00"); // Ctrl+Space
    app.send(b"p");
    assert!(app.wait_for("Commands"), "{}", app.screen());
    app.send(b"toggle agents");
    assert!(app.wait_for("Toggle Agents Panel"), "{}", app.screen());
    app.send(b"\x1b"); // Esc
    assert!(
        app.wait_gone("Toggle Agents Panel"),
        "palette did not close:\n{}",
        app.screen()
    );

    // 6. Quit cleanly with Ctrl+Q.
    app.send(b"\x11");
    assert!(app.wait_exit(), "app did not exit:\n{}", app.screen());
}

#[test]
fn starts_outside_a_git_repository_and_survives_resize() {
    let mut app = Harness::start();
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());
    // Resizing the pty must not crash the workbench.
    app.send(b"\x00");
    app.send(b"b"); // toggle explorer off
    assert!(app.wait_for("EDITOR"), "{}", app.screen());
    app.send(b"\x11");
    assert!(app.wait_exit(), "app did not exit:\n{}", app.screen());
}
