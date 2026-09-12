//! Validation of the optional Herdr backend against the real CLI.
//!
//! Herdr is never required: when the binary is missing or no server is
//! running, the test reports that and returns, exactly as the workbench falls
//! back to local PTY agents. Nothing here creates, writes to, or stops a pane
//! in the user's session — it only reads.

use std::time::Duration;

use termloom::domain::agent::{AgentBackendKind, ObservationSource};
use termloom::services::agents::{AgentBackend, HerdrAgentBackend};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn herdr_installed() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("herdr").is_file()))
        .unwrap_or(false)
}

#[test]
fn connects_to_a_running_herdr_and_normalises_its_agents() {
    if !herdr_installed() {
        eprintln!("skipping: herdr is not installed");
        return;
    }
    let runtime = runtime();
    let backend = match runtime.block_on(HerdrAgentBackend::connect(None)) {
        Ok(backend) => backend,
        Err(error) => {
            // No server running is a normal, supported situation.
            eprintln!("skipping: {error:#}");
            return;
        }
    };

    let capabilities = backend.capabilities();
    assert_eq!(backend.backend_kind(), AgentBackendKind::Herdr);
    assert!(
        !capabilities.spawn,
        "the adapter must not create panes in the user's layout"
    );
    assert!(capabilities.snapshot);
    assert!(
        backend.status().to_lowercase().contains("herdr"),
        "{}",
        backend.status()
    );

    let agents = runtime
        .block_on(backend.list_agents())
        .expect("listing agents should succeed against a live server");

    for agent in &agents {
        assert_eq!(agent.backend, AgentBackendKind::Herdr);
        assert_eq!(
            agent.state_source,
            ObservationSource::Backend,
            "Herdr states are reported, not guessed"
        );
        assert!(
            agent.backend_session_id.is_some(),
            "every Herdr agent keeps its pane identity"
        );
        assert!(!agent.label.is_empty());
    }

    // Reading an agent's output must not disturb it.
    if let Some(agent) = agents.first() {
        let snapshot = runtime
            .block_on(backend.snapshot(agent.id))
            .expect("reading a pane snapshot should succeed");
        assert!(!snapshot.unavailable);
    }

    // A refresh keeps identities stable so the dashboard does not flicker.
    runtime.block_on(backend.refresh()).unwrap();
    let again = runtime.block_on(backend.list_agents()).unwrap();
    let before: Vec<_> = agents.iter().map(|agent| agent.id).collect();
    let after: Vec<_> = again.iter().map(|agent| agent.id).collect();
    assert_eq!(before, after, "agent ids must be stable across refreshes");
}

#[test]
fn a_missing_herdr_binary_degrades_instead_of_failing_the_workbench() {
    let runtime = runtime();
    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(10),
            HerdrAgentBackend::connect(Some("definitely-not-herdr")),
        )
        .await
    });
    let connection = result.expect("connecting must not hang");
    match connection {
        Ok(_) => panic!("a missing binary cannot connect"),
        Err(error) => assert!(
            !format!("{error:#}").is_empty(),
            "the failure should explain itself"
        ),
    }
}
