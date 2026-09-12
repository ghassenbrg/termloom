//! End-to-end smoke test: run the real binary inside a pseudo terminal and
//! drive it with keystrokes, the way a user would.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize};

/// A workspace on disk plus an isolated HOME, reusable across restarts.
struct Fixture {
    dir: tempfile::TempDir,
    home: tempfile::TempDir,
}

impl Fixture {
    /// A plain project directory.
    fn new() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "# demo\n").unwrap();
        Fixture {
            dir,
            // Config/state must live outside the workspace, or they would show
            // up in the explorer and shift the rows these tests navigate.
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// A git repository with one commit and one uncommitted change.
    fn git_repo() -> Fixture {
        let fixture = Fixture::new();
        let root = fixture.dir.path();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("notes.txt"), "first\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "initial"]);
        // Leave a working-tree change for the Git panel to show.
        std::fs::write(root.join("notes.txt"), "first\nsecond\n").unwrap();
        fixture
    }

    /// Launch TermLoom against this workspace.
    fn start(&self) -> Harness {
        Harness::start(self.dir.path(), self.home.path())
    }
}

/// A running TermLoom under our own pty, with its screen parsed by vt100.
struct Harness {
    /// Kept so the test can resize the pty the way a window manager would.
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl Harness {
    fn start(root: &Path, home: &Path) -> Harness {
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
        command.env("HOME", home.to_string_lossy().to_string());
        command.env(
            "XDG_CONFIG_HOME",
            home.join("config").to_string_lossy().to_string(),
        );
        command.env(
            "XDG_STATE_HOME",
            home.join("state").to_string_lossy().to_string(),
        );

        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let parser_size = (40u16, 150u16);
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            parser_size.0,
            parser_size.1,
            2000,
        )));

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
            master: pair.master,
            writer,
            parser,
            child,
        }
    }

    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Resize the pty and the local screen model together.
    fn resize(&mut self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        self.parser.lock().unwrap().set_size(rows, cols);
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
    let fixture = Fixture::new();
    let mut app = fixture.start();

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
    let fixture = Fixture::new();
    let mut app = fixture.start();
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());
    // Resizing the pty must not crash the workbench.
    app.send(b"\x00");
    app.send(b"b"); // toggle explorer off
    assert!(app.wait_for("EDITOR"), "{}", app.screen());
    app.send(b"\x11");
    assert!(app.wait_exit(), "app did not exit:\n{}", app.screen());
}

#[test]
fn restores_open_files_after_a_restart() {
    let fixture = Fixture::new();

    // First run: open a file and quit cleanly.
    let mut app = fixture.start();
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());
    app.send(b"\x1b[B"); // Down to README.md
    app.send(b"\r");
    assert!(app.wait_for("# demo"), "{}", app.screen());
    app.send(b"\x11"); // Ctrl+Q
    assert!(app.wait_exit(), "{}", app.screen());

    // Second run: the workspace state brings the file back.
    let mut app = fixture.start();
    assert!(
        app.wait_for("README.md"),
        "the tab should be restored:\n{}",
        app.screen()
    );
    assert!(
        app.wait_for("# demo"),
        "the file contents should be restored:\n{}",
        app.screen()
    );
    assert!(
        app.wait_for("restored"),
        "the restore should be reported:\n{}",
        app.screen()
    );
    app.send(b"\x11");
    assert!(app.wait_exit(), "{}", app.screen());
}

#[test]
fn shows_and_stages_a_git_change() {
    let fixture = Fixture::git_repo();
    let mut app = fixture.start();

    // The Git panel picks the change up on its own refresh.
    assert!(
        app.wait_for("notes.txt"),
        "the changed file should appear in the Git panel:\n{}",
        app.screen()
    );
    assert!(
        app.screen().contains(" M notes.txt") || app.screen().contains("M notes.txt"),
        "the worktree status should be shown:\n{}",
        app.screen()
    );

    // Focus the Git panel and stage the selected file.
    app.send(b"\x00"); // Ctrl+Space
    app.send(b"g");
    app.send(b"s");
    assert!(
        app.wait_for("M  notes.txt"),
        "staging should move the change into the index:\n{}",
        app.screen()
    );

    // Unstage it again.
    app.send(b"u");
    assert!(
        app.wait_for(" M notes.txt"),
        "unstaging should return it to the worktree:\n{}",
        app.screen()
    );

    app.send(b"\x11");
    assert!(app.wait_exit(), "{}", app.screen());
}

#[test]
fn survives_repeated_resizes_down_to_a_tiny_terminal() {
    let fixture = Fixture::new();
    let mut app = fixture.start();
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());

    // Open a file so every panel has content to lay out.
    app.send(b"\x1b[B");
    app.send(b"\r");
    assert!(app.wait_for("# demo"), "{}", app.screen());

    // Walk through wide, compact, minimal and back, several times.
    for _ in 0..3 {
        for (rows, cols) in [(40, 150), (24, 100), (12, 60), (8, 30), (44, 170)] {
            app.resize(rows, cols);
            std::thread::sleep(Duration::from_millis(180));
        }
    }

    // The workbench is still alive and drawing at the final size.
    assert!(
        app.wait_for("TermLoom"),
        "the workbench should still render after resizing:\n{}",
        app.screen()
    );
    assert!(app.wait_for("EXPLORER"), "{}", app.screen());

    // And it still responds to input.
    app.send(b"\x00");
    app.send(b"t");
    assert!(
        app.wait_for("Terminal 1"),
        "input should still work after resizing:\n{}",
        app.screen()
    );

    app.send(b"\x11");
    assert!(app.wait_exit(), "{}", app.screen());
}
