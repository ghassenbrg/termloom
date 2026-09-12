//! The command registry.
//!
//! Everything the workbench can do has a stable id (`agent.new.claude`).
//! Keybindings, the palette and menu hints all target ids, never functions, so
//! a rebind or a new entry point cannot drift away from the implementation.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

/// Grouping used as a prefix in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    File,
    Editor,
    View,
    Terminal,
    Agent,
    Git,
    Language,
    Debug,
    Extensions,
    Workspace,
    Application,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::File => "File",
            Category::Editor => "Editor",
            Category::View => "View",
            Category::Terminal => "Terminal",
            Category::Agent => "Agent",
            Category::Git => "Git",
            Category::Language => "Language",
            Category::Debug => "Debug",
            Category::Extensions => "Extensions",
            Category::Workspace => "Workspace",
            Category::Application => "TermLoom",
        }
    }
}

/// Precondition for a command to be runnable. The palette greys out anything
/// unavailable instead of failing after the user picks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    Always,
    ActiveEditor,
    DirtyEditor,
    GitRepository,
    SelectedGitFile,
    SelectedAgent,
    ActiveTerminal,
    LanguageServer,
    DebugSession,
}

/// A registry entry.
#[derive(Debug, Clone, Copy)]
pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub category: Category,
    pub requires: Requirement,
}

impl Command {
    /// `Agent: New Claude Session` — what the palette shows.
    pub fn display(&self) -> String {
        format!("{}: {}", self.category.label(), self.title)
    }
}

macro_rules! command {
    ($id:literal, $title:literal, $category:ident, $requires:ident) => {
        Command {
            id: $id,
            title: $title,
            category: Category::$category,
            requires: Requirement::$requires,
        }
    };
}

/// Every command TermLoom knows about.
pub const COMMANDS: &[Command] = &[
    // File
    command!("file.save", "Save", File, DirtyEditor),
    command!("file.save_all", "Save All", File, Always),
    command!("file.reload", "Reload File From Disk", File, ActiveEditor),
    command!("file.new", "New File", File, Always),
    command!("file.new_folder", "New Folder", File, Always),
    command!("file.rename", "Rename…", File, Always),
    command!("file.delete", "Delete…", File, Always),
    command!(
        "file.reveal",
        "Reveal Active File In Explorer",
        File,
        ActiveEditor
    ),
    // Editor
    command!("editor.close_tab", "Close Tab", Editor, ActiveEditor),
    command!(
        "editor.close_others",
        "Close Other Tabs",
        Editor,
        ActiveEditor
    ),
    command!("editor.next_tab", "Next Tab", Editor, ActiveEditor),
    command!("editor.prev_tab", "Previous Tab", Editor, ActiveEditor),
    command!("editor.reopen_tab", "Reopen Closed Tab", Editor, Always),
    command!("editor.undo", "Undo", Editor, ActiveEditor),
    command!("editor.redo", "Redo", Editor, ActiveEditor),
    command!("editor.find", "Find In File", Editor, ActiveEditor),
    command!("editor.find_next", "Find Next", Editor, ActiveEditor),
    command!("editor.find_prev", "Find Previous", Editor, ActiveEditor),
    command!("editor.goto_line", "Go To Line…", Editor, ActiveEditor),
    command!("editor.copy", "Copy", Editor, ActiveEditor),
    command!("editor.cut", "Cut", Editor, ActiveEditor),
    command!("editor.paste", "Paste", Editor, ActiveEditor),
    command!("editor.select_all", "Select All", Editor, ActiveEditor),
    command!(
        "editor.toggle_comment",
        "Toggle Line Comment",
        Editor,
        ActiveEditor
    ),
    // View
    command!("view.toggle.explorer", "Toggle Explorer", View, Always),
    command!("view.toggle.outline", "Toggle Outline", View, Always),
    command!("view.toggle.git", "Toggle Git Panel", View, Always),
    command!("view.toggle.agents", "Toggle Agents Panel", View, Always),
    command!(
        "view.toggle.terminals",
        "Toggle Terminal Strip",
        View,
        Always
    ),
    command!(
        "view.toggle.problems",
        "Toggle Problems Panel",
        View,
        Always
    ),
    command!("view.toggle.debug", "Toggle Debug Panel", View, Always),
    command!("view.toggle.extensions", "Show Extensions", View, Always),
    command!("view.focus.explorer", "Focus Explorer", View, Always),
    command!("view.focus.outline", "Focus Outline", View, Always),
    command!("view.focus.git", "Focus Git", View, Always),
    command!("view.focus.agents", "Focus Agents", View, Always),
    command!("view.focus.editor", "Focus Editor", View, ActiveEditor),
    command!(
        "view.focus.terminal",
        "Focus Terminal",
        View,
        ActiveTerminal
    ),
    command!("focus.left", "Focus Panel Left", View, Always),
    command!("focus.right", "Focus Panel Right", View, Always),
    command!("focus.up", "Focus Panel Up", View, Always),
    command!("focus.down", "Focus Panel Down", View, Always),
    // Terminal
    command!("terminal.new", "New Terminal", Terminal, Always),
    command!("terminal.next", "Next Terminal", Terminal, ActiveTerminal),
    command!(
        "terminal.prev",
        "Previous Terminal",
        Terminal,
        ActiveTerminal
    ),
    command!("terminal.kill", "Kill Terminal", Terminal, ActiveTerminal),
    command!(
        "terminal.restart",
        "Restart Terminal",
        Terminal,
        ActiveTerminal
    ),
    command!(
        "terminal.close",
        "Close Terminal Pane",
        Terminal,
        ActiveTerminal
    ),
    command!(
        "terminal.clear_scroll",
        "Scroll Terminal To Bottom",
        Terminal,
        ActiveTerminal
    ),
    // Agents
    command!("agent.new.claude", "New Claude Session", Agent, Always),
    command!("agent.new.codex", "New Codex Session", Agent, Always),
    command!("agent.new.shell", "New Shell Session", Agent, Always),
    command!("agent.new.custom", "New Agent From Command…", Agent, Always),
    command!(
        "agent.focus_terminal",
        "Focus Agent Terminal",
        Agent,
        SelectedAgent
    ),
    command!(
        "agent.send_input",
        "Send Text To Agent…",
        Agent,
        SelectedAgent
    ),
    command!("agent.stop", "Stop Agent", Agent, SelectedAgent),
    command!("agent.restart", "Restart Agent", Agent, SelectedAgent),
    command!("agent.rename", "Rename Agent…", Agent, SelectedAgent),
    command!(
        "agent.remove",
        "Remove Agent From Dashboard",
        Agent,
        SelectedAgent
    ),
    command!("agent.task.add", "Add Agent Task…", Agent, SelectedAgent),
    command!(
        "agent.task.toggle",
        "Toggle Agent Task",
        Agent,
        SelectedAgent
    ),
    // Git
    command!("git.refresh", "Refresh Status", Git, GitRepository),
    command!("git.stage", "Stage File", Git, SelectedGitFile),
    command!("git.unstage", "Unstage File", Git, SelectedGitFile),
    command!("git.stage_all", "Stage All Changes", Git, GitRepository),
    command!("git.unstage_all", "Unstage All Changes", Git, GitRepository),
    command!("git.discard", "Discard Changes…", Git, SelectedGitFile),
    command!("git.open_diff", "Open Diff", Git, SelectedGitFile),
    command!("git.open_file", "Open Changed File", Git, SelectedGitFile),
    // Language services
    command!("lsp.start", "Start Language Server", Language, ActiveEditor),
    command!(
        "lsp.restart",
        "Restart Language Server",
        Language,
        LanguageServer
    ),
    command!("lsp.stop", "Stop Language Server", Language, LanguageServer),
    command!("lsp.hover", "Show Hover", Language, LanguageServer),
    command!(
        "lsp.completion",
        "Trigger Completion",
        Language,
        ActiveEditor
    ),
    command!(
        "lsp.definition",
        "Go To Definition",
        Language,
        LanguageServer
    ),
    command!(
        "lsp.references",
        "Find References",
        Language,
        LanguageServer
    ),
    command!("lsp.rename", "Rename Symbol…", Language, LanguageServer),
    command!("lsp.format", "Format Document", Language, LanguageServer),
    command!("lsp.back", "Go Back", Language, Always),
    // Debug
    command!("debug.start", "Start Debugging", Debug, Always),
    command!("debug.stop", "Stop Debugging", Debug, DebugSession),
    command!("debug.continue", "Continue", Debug, DebugSession),
    command!("debug.pause", "Pause", Debug, DebugSession),
    command!("debug.step_over", "Step Over", Debug, DebugSession),
    command!("debug.step_in", "Step Into", Debug, DebugSession),
    command!("debug.step_out", "Step Out", Debug, DebugSession),
    command!(
        "debug.evaluate",
        "Evaluate Expression…",
        Debug,
        DebugSession
    ),
    command!(
        "debug.toggle_breakpoint",
        "Toggle Breakpoint",
        Debug,
        ActiveEditor
    ),
    command!(
        "debug.clear_breakpoints",
        "Remove All Breakpoints",
        Debug,
        Always
    ),
    // Extensions
    command!(
        "extensions.list",
        "List Installed Extensions",
        Extensions,
        Always
    ),
    command!("extensions.install", "Install VSIX…", Extensions, Always),
    command!("extensions.inspect", "Inspect VSIX…", Extensions, Always),
    command!("extensions.remove", "Remove Extension", Extensions, Always),
    // Workspace / app
    command!("palette.commands", "Command Palette", Workspace, Always),
    command!("palette.quick_open", "Quick Open File", Workspace, Always),
    command!("workspace.reload", "Reload Workspace", Workspace, Always),
    command!("workspace.run_task", "Run Task…", Workspace, Always),
    command!(
        "workspace.open_config",
        "Open Configuration File",
        Workspace,
        Always
    ),
    command!("help.toggle", "Keyboard Shortcuts", Application, Always),
    command!("app.quit", "Quit TermLoom", Application, Always),
    command!("workbench.prefix", "Command Prefix", Application, Always),
];

/// A fuzzy-matched command with its score.
#[derive(Debug, Clone, Copy)]
pub struct CommandMatch {
    pub command: Command,
    pub score: u32,
}

/// Lookup and fuzzy search over [`COMMANDS`].
pub struct CommandRegistry {
    matcher: Matcher,
}

impl CommandRegistry {
    pub fn new() -> CommandRegistry {
        CommandRegistry {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    pub fn all(&self) -> &'static [Command] {
        COMMANDS
    }

    /// Exact lookup by id.
    pub fn get(&self, id: &str) -> Option<Command> {
        COMMANDS.iter().find(|c| c.id == id).copied()
    }

    /// Fuzzy search over `Category: Title` plus the raw id, best first.
    pub fn search(&mut self, query: &str) -> Vec<CommandMatch> {
        if query.trim().is_empty() {
            return COMMANDS
                .iter()
                .map(|command| CommandMatch {
                    command: *command,
                    score: 0,
                })
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut scored: Vec<CommandMatch> = COMMANDS
            .iter()
            .filter_map(|command| {
                let haystack = format!("{} {}", command.display(), command.id);
                let mut buf = Vec::new();
                let utf32 = nucleo_matcher::Utf32Str::new(&haystack, &mut buf);
                pattern
                    .score(utf32, &mut self.matcher)
                    .map(|score| CommandMatch {
                        command: *command,
                        score,
                    })
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.command.display().cmp(&b.command.display()))
        });
        scored
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        CommandRegistry::new()
    }
}

/// Fuzzy-rank arbitrary strings (used by quick open for file paths).
pub fn fuzzy_rank<'a>(
    matcher: &mut Matcher,
    query: &str,
    items: impl Iterator<Item = &'a str>,
    limit: usize,
) -> Vec<(&'a str, u32)> {
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut scored: Vec<(&str, u32)> = items
        .filter_map(|item| {
            let mut buf = Vec::new();
            let utf32 = nucleo_matcher::Utf32Str::new(item, &mut buf);
            pattern.score(utf32, matcher).map(|score| (item, score))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.len().cmp(&b.0.len())));
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn command_ids_are_unique() {
        let mut seen = HashSet::new();
        for command in COMMANDS {
            assert!(
                seen.insert(command.id),
                "duplicate command id {}",
                command.id
            );
        }
    }

    #[test]
    fn required_v1_commands_exist() {
        let registry = CommandRegistry::new();
        for id in [
            "agent.new.claude",
            "agent.new.codex",
            "terminal.new",
            "file.save",
            "file.save_all",
            "view.toggle.explorer",
            "view.toggle.git",
            "view.toggle.agents",
            "workspace.reload",
        ] {
            assert!(registry.get(id).is_some(), "missing command {id}");
        }
    }

    #[test]
    fn every_default_binding_targets_a_real_command() {
        let registry = CommandRegistry::new();
        for (chord, id, _) in crate::config::keys::DEFAULT_BINDINGS {
            assert!(
                registry.get(id).is_some(),
                "binding {chord} points at unknown command {id}"
            );
        }
    }

    #[test]
    fn fuzzy_search_ranks_the_obvious_match_first() {
        let mut registry = CommandRegistry::new();
        let results = registry.search("new claude");
        assert_eq!(results[0].command.id, "agent.new.claude");

        let results = registry.search("togexpl");
        assert_eq!(results[0].command.id, "view.toggle.explorer");
    }

    #[test]
    fn search_can_match_by_command_id() {
        let mut registry = CommandRegistry::new();
        let results = registry.search("git.discard");
        assert_eq!(results[0].command.id, "git.discard");
    }

    #[test]
    fn empty_query_lists_everything() {
        let mut registry = CommandRegistry::new();
        assert_eq!(registry.search("  ").len(), COMMANDS.len());
    }

    #[test]
    fn nonsense_query_matches_nothing() {
        let mut registry = CommandRegistry::new();
        assert!(registry.search("zzzqqqxxx").is_empty());
    }

    #[test]
    fn file_ranking_prefers_shorter_paths_on_ties() {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let files = [
            "src/app/auth_provider.rs",
            "test/auth/auth_provider_test.rs",
            "docs/readme.md",
        ];
        let ranked = fuzzy_rank(&mut matcher, "authprov", files.iter().copied(), 10);
        assert_eq!(ranked[0].0, "src/app/auth_provider.rs");
        assert_eq!(ranked.len(), 2, "unrelated files are filtered out");
    }
}
