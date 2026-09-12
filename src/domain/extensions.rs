//! Extension compatibility model.
//!
//! TermLoom never runs VS Code extension code. It inspects a package, decides
//! which declarative pieces it can actually use, and reports the rest as
//! explicitly unsupported. Compatibility is capability-based: installation
//! succeeding says nothing about support.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Overall verdict for a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompatibilityClass {
    /// Every capability the package contributes is supported.
    Full,
    /// Some capabilities work; the rest are listed.
    Partial,
    /// Nothing portable can be used.
    Unsupported,
    /// An expected capability failed validation.
    Broken,
}

impl CompatibilityClass {
    pub fn label(self) -> &'static str {
        match self {
            CompatibilityClass::Full => "Full",
            CompatibilityClass::Partial => "Partial",
            CompatibilityClass::Unsupported => "Unsupported",
            CompatibilityClass::Broken => "Broken",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            CompatibilityClass::Full => "✓",
            CompatibilityClass::Partial => "◐",
            CompatibilityClass::Unsupported => "✕",
            CompatibilityClass::Broken => "!",
        }
    }
}

/// How well one contribution type is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilitySupport {
    /// Usable as-is.
    Supported,
    /// Usable after explicit user configuration or consent.
    NeedsAdaptation,
    /// Cannot work in a terminal-native workbench.
    Unsupported,
}

impl CapabilitySupport {
    pub fn glyph(self) -> &'static str {
        match self {
            CapabilitySupport::Supported => "✓",
            CapabilitySupport::NeedsAdaptation => "◐",
            CapabilitySupport::Unsupported => "✕",
        }
    }
}

/// One classified contribution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReport {
    /// Manifest key, e.g. `contributes.grammars`.
    pub name: String,
    pub support: CapabilitySupport,
    /// Why it is (un)supported — always shown in the UI.
    pub detail: String,
}

/// The result of inspecting a VSIX or an installed extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// `publisher.name`.
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub publisher: String,
    pub class: CompatibilityClass,
    pub capabilities: Vec<CapabilityReport>,
    /// Declarative assets TermLoom can load.
    pub assets: Vec<ExtensionAsset>,
    /// Where the package is installed, when it is.
    pub install_path: Option<PathBuf>,
}

impl CompatibilityReport {
    /// Capabilities of a given support level.
    pub fn of(&self, support: CapabilitySupport) -> Vec<&CapabilityReport> {
        self.capabilities
            .iter()
            .filter(|c| c.support == support)
            .collect()
    }

    /// Derive the overall class from the individual capabilities.
    pub fn classify(capabilities: &[CapabilityReport]) -> CompatibilityClass {
        if capabilities.is_empty() {
            return CompatibilityClass::Unsupported;
        }
        let supported = capabilities
            .iter()
            .filter(|c| c.support == CapabilitySupport::Supported)
            .count();
        let adaptable = capabilities
            .iter()
            .filter(|c| c.support == CapabilitySupport::NeedsAdaptation)
            .count();
        let unsupported = capabilities
            .iter()
            .filter(|c| c.support == CapabilitySupport::Unsupported)
            .count();
        if supported == 0 && adaptable == 0 {
            CompatibilityClass::Unsupported
        } else if unsupported == 0
            && capabilities
                .iter()
                .all(|c| c.support == CapabilitySupport::Supported)
        {
            CompatibilityClass::Full
        } else {
            CompatibilityClass::Partial
        }
    }
}

/// A declarative asset TermLoom can consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtensionAsset {
    Language {
        id: String,
        extensions: Vec<String>,
        filenames: Vec<String>,
        configuration: Option<PathBuf>,
    },
    Grammar {
        language: Option<String>,
        scope_name: String,
        path: PathBuf,
    },
    Snippets {
        language: String,
        path: PathBuf,
    },
    Theme {
        label: String,
        dark: bool,
        path: PathBuf,
    },
    /// Debugger metadata; the adapter is only run after explicit consent.
    Debugger {
        type_name: String,
        label: String,
        program: Option<PathBuf>,
        runtime: Option<String>,
    },
}

impl ExtensionAsset {
    pub fn label(&self) -> String {
        match self {
            ExtensionAsset::Language { id, .. } => format!("language {id}"),
            ExtensionAsset::Grammar { scope_name, .. } => format!("grammar {scope_name}"),
            ExtensionAsset::Snippets { language, .. } => format!("snippets {language}"),
            ExtensionAsset::Theme { label, .. } => format!("theme {label}"),
            ExtensionAsset::Debugger { type_name, .. } => format!("debugger {type_name}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(support: CapabilitySupport) -> CapabilityReport {
        CapabilityReport {
            name: "contributes.grammars".into(),
            support,
            detail: "test".into(),
        }
    }

    #[test]
    fn all_supported_is_full() {
        let caps = vec![
            capability(CapabilitySupport::Supported),
            capability(CapabilitySupport::Supported),
        ];
        assert_eq!(
            CompatibilityReport::classify(&caps),
            CompatibilityClass::Full
        );
    }

    #[test]
    fn a_mix_is_partial() {
        let caps = vec![
            capability(CapabilitySupport::Supported),
            capability(CapabilitySupport::Unsupported),
        ];
        assert_eq!(
            CompatibilityReport::classify(&caps),
            CompatibilityClass::Partial
        );
    }

    #[test]
    fn nothing_supported_is_unsupported() {
        let caps = vec![capability(CapabilitySupport::Unsupported)];
        assert_eq!(
            CompatibilityReport::classify(&caps),
            CompatibilityClass::Unsupported
        );
        assert_eq!(
            CompatibilityReport::classify(&[]),
            CompatibilityClass::Unsupported
        );
    }

    #[test]
    fn adaptation_only_is_partial_not_full() {
        let caps = vec![capability(CapabilitySupport::NeedsAdaptation)];
        assert_eq!(
            CompatibilityReport::classify(&caps),
            CompatibilityClass::Partial
        );
    }
}
