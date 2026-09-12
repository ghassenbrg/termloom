//! Explicit focus model.
//!
//! Input routing differs completely between the editor, a child PTY and the
//! command palette, so focus is stored, never inferred from draw order.

use crate::domain::ids::{EditorTabId, TerminalId};

/// The surface that currently receives keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Explorer,
    Outline,
    Git,
    Editor(EditorTabId),
    AgentList,
    AgentDetail,
    Terminal(TerminalId),
    Problems,
    Debug,
    CommandPalette,
    /// A blocking dialog owns the keyboard.
    Modal,
}

impl FocusTarget {
    /// Name shown in the status bar.
    pub fn label(self) -> &'static str {
        match self {
            FocusTarget::Explorer => "EXPLORER",
            FocusTarget::Outline => "OUTLINE",
            FocusTarget::Git => "GIT",
            FocusTarget::Editor(_) => "EDITOR",
            FocusTarget::AgentList => "AGENTS",
            FocusTarget::AgentDetail => "AGENT",
            FocusTarget::Terminal(_) => "TERMINAL",
            FocusTarget::Problems => "PROBLEMS",
            FocusTarget::Debug => "DEBUG",
            FocusTarget::CommandPalette => "PALETTE",
            FocusTarget::Modal => "DIALOG",
        }
    }

    /// True when keystrokes belong to a child process.
    pub fn is_terminal(self) -> bool {
        matches!(self, FocusTarget::Terminal(_))
    }

    pub fn is_editor(self) -> bool {
        matches!(self, FocusTarget::Editor(_))
    }

    /// True for list-style panels that share navigation keys.
    pub fn is_panel(self) -> bool {
        matches!(
            self,
            FocusTarget::Explorer
                | FocusTarget::Outline
                | FocusTarget::Git
                | FocusTarget::AgentList
                | FocusTarget::AgentDetail
                | FocusTarget::Problems
                | FocusTarget::Debug
        )
    }

    /// Which screen region the target lives in, used by directional focus
    /// movement (`Ctrl+Space` then an arrow key).
    pub fn region(self) -> Region {
        match self {
            FocusTarget::Explorer | FocusTarget::Outline | FocusTarget::Git => Region::Sidebar,
            FocusTarget::Editor(_) | FocusTarget::Problems | FocusTarget::Debug => Region::Center,
            FocusTarget::AgentList | FocusTarget::AgentDetail => Region::Agents,
            FocusTarget::Terminal(_) => Region::Terminals,
            FocusTarget::CommandPalette | FocusTarget::Modal => Region::Overlay,
        }
    }
}

/// Coarse screen regions of the workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Sidebar,
    Center,
    Agents,
    Terminals,
    Overlay,
}

/// Direction for `focus.left` / `focus.right` / `focus.up` / `focus.down`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Where a directional move should land, given the visible regions.
///
/// Layout (large terminal):
///
/// ```text
/// ┌──────────┬───────────┬─────────┐
/// │ Sidebar  │  Center   │ Agents  │
/// ├──────────┴───────────┴─────────┤
/// │           Terminals            │
/// └────────────────────────────────┘
/// ```
pub fn next_region(from: Region, direction: Direction, visible: &[Region]) -> Option<Region> {
    let order = [Region::Sidebar, Region::Center, Region::Agents];
    let candidate = match direction {
        Direction::Left | Direction::Right => {
            if from == Region::Terminals {
                // Horizontal movement inside the terminal strip is handled by
                // the terminal panel itself.
                return None;
            }
            let index = order.iter().position(|r| *r == from)?;
            let step: isize = if direction == Direction::Left { -1 } else { 1 };
            let mut next = index as isize + step;
            while (0..order.len() as isize).contains(&next) {
                let region = order[next as usize];
                if visible.contains(&region) {
                    return Some(region);
                }
                next += step;
            }
            return None;
        }
        Direction::Down => {
            if from == Region::Terminals {
                return None;
            }
            Region::Terminals
        }
        Direction::Up => {
            if from == Region::Terminals {
                Region::Center
            } else {
                return None;
            }
        }
    };
    visible.contains(&candidate).then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_focus_is_recognised() {
        let target = FocusTarget::Terminal(TerminalId::next());
        assert!(target.is_terminal());
        assert!(!target.is_editor());
        assert_eq!(target.region(), Region::Terminals);
    }

    #[test]
    fn horizontal_movement_skips_hidden_regions() {
        let visible = [Region::Sidebar, Region::Center, Region::Terminals];
        assert_eq!(
            next_region(Region::Sidebar, Direction::Right, &visible),
            Some(Region::Center)
        );
        // Agents hidden: moving right from the centre goes nowhere.
        assert_eq!(
            next_region(Region::Center, Direction::Right, &visible),
            None
        );

        let all = [Region::Sidebar, Region::Center, Region::Agents];
        assert_eq!(
            next_region(Region::Center, Direction::Right, &all),
            Some(Region::Agents)
        );
        assert_eq!(
            next_region(Region::Agents, Direction::Left, &all),
            Some(Region::Center)
        );
    }

    #[test]
    fn vertical_movement_reaches_the_terminal_strip_and_back() {
        let visible = [Region::Sidebar, Region::Center, Region::Terminals];
        assert_eq!(
            next_region(Region::Center, Direction::Down, &visible),
            Some(Region::Terminals)
        );
        assert_eq!(
            next_region(Region::Terminals, Direction::Up, &visible),
            Some(Region::Center)
        );
        let no_terminals = [Region::Sidebar, Region::Center];
        assert_eq!(
            next_region(Region::Center, Direction::Down, &no_terminals),
            None
        );
    }
}
