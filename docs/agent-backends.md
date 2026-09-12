# Agent backends

Every agent is normalized to `Starting`, `Working`, `WaitingForInput`, `Idle`,
`Done`, `Failed`, `Exited`, or `Unknown`, with source and confidence. The UI
does not infer a model's task or claim ownership of changed files.

`LocalAgentBackend` launches configured programs in real PTYs. Process exit is
authoritative; recent activity and conservative prompt markers are lower
confidence. Claude and Codex presets are built in, and shell/custom commands
remain available without those CLIs.

`HerdrAgentBackend` maps Herdr's `working`, `blocked`, `idle`, `done`, and
`unknown` states, reads recent output, submits explicit prompts, and requests
stop, rename, restart, and focus. It has no embedded local PTY because Herdr
owns that pane. Local spawning remains primary so TermLoom stays independent.
