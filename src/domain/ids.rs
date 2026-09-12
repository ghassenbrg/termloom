//! Opaque identifiers for workbench objects.
//!
//! Ids are process-local, monotonic and cheap to copy. They are deliberately
//! *not* indexes into a `Vec`: collections can be reordered or compacted
//! without invalidating references held elsewhere in the state tree.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        pub struct $name(pub u64);

        impl $name {
            /// Allocate a fresh, process-unique id.
            pub fn next() -> Self {
                static COUNTER: AtomicU64 = AtomicU64::new(1);
                Self(COUNTER.fetch_add(1, Ordering::Relaxed))
            }

            /// Raw numeric value, useful for stable display (`Terminal 3`).
            pub fn raw(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0)
            }
        }
    };
}

define_id!(
    /// Identifies an editor tab (one open buffer view).
    EditorTabId, "tab:"
);
define_id!(
    /// Identifies a terminal session (one PTY + child process).
    TerminalId, "term:"
);
define_id!(
    /// Identifies an agent session (a terminal plus agent semantics).
    AgentId, "agent:"
);
define_id!(
    /// Identifies a modal dialog instance.
    ModalId, "modal:"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_monotonic() {
        let a = TerminalId::next();
        let b = TerminalId::next();
        assert!(b.raw() > a.raw());
        assert_ne!(a, b);
    }

    #[test]
    fn id_counters_are_per_type() {
        // Distinct types have distinct counters; this mostly documents intent.
        let t = EditorTabId::next();
        assert!(t.to_string().starts_with("tab:"));
    }
}
