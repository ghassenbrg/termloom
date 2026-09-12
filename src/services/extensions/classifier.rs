//! Manifest capability classification.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::domain::extensions::{
    CapabilityReport, CapabilitySupport, CompatibilityClass, CompatibilityReport, ExtensionAsset,
};

use super::manifest::ExtensionManifest;

/// Turn a manifest into the exact compatibility statement shown to users.
///
/// `asset_exists` is deliberately supplied by the package reader: archive and
/// installed-directory inspection use the same policy without coupling the
/// domain report to ZIP or filesystem types.
pub fn classify(
    manifest: &ExtensionManifest,
    install_path: Option<PathBuf>,
    asset_exists: impl Fn(&Path) -> bool,
) -> CompatibilityReport {
    let mut capabilities = Vec::new();
    let mut assets = Vec::new();
    let mut broken = false;

    for language in manifest.languages() {
        let configuration_ok = language.configuration.as_deref().is_none_or(&asset_exists);
        broken |= !configuration_ok;
        capabilities.push(CapabilityReport {
            name: format!("language {}", language.id),
            support: if configuration_ok {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            },
            detail: if configuration_ok {
                "file associations load natively; any language configuration file is preserved for an explicit adapter".into()
            } else {
                "the referenced language configuration is missing or unsafe".into()
            },
        });
        if configuration_ok {
            assets.push(ExtensionAsset::Language {
                id: language.id,
                extensions: language.extensions,
                filenames: language.filenames,
                configuration: language.configuration,
            });
        }
    }

    for grammar in manifest.grammars() {
        let valid = safe_relative(&grammar.path) && asset_exists(&grammar.path);
        broken |= !valid;
        capabilities.push(asset_capability(
            format!("grammar {}", grammar.scope_name),
            valid,
            CapabilitySupport::NeedsAdaptation,
            "TextMate grammar is installed safely; the V1 editor uses built-in Tree-sitter grammars and requires an explicit grammar adapter",
        ));
        if valid {
            assets.push(ExtensionAsset::Grammar {
                language: grammar.language,
                scope_name: grammar.scope_name,
                path: grammar.path,
            });
        }
    }

    for snippets in manifest.snippets() {
        let valid = safe_relative(&snippets.path) && asset_exists(&snippets.path);
        broken |= !valid;
        capabilities.push(asset_capability(
            format!("snippets {}", snippets.language),
            valid,
            CapabilitySupport::Supported,
            "snippet definitions are portable declarative data",
        ));
        if valid {
            assets.push(ExtensionAsset::Snippets {
                language: snippets.language,
                path: snippets.path,
            });
        }
    }

    for theme in manifest.themes() {
        let valid = safe_relative(&theme.path) && asset_exists(&theme.path);
        broken |= !valid;
        capabilities.push(asset_capability(
            format!("theme {}", theme.label),
            valid,
            CapabilitySupport::Supported,
            "color theme is available as declarative data",
        ));
        if valid {
            assets.push(ExtensionAsset::Theme {
                label: theme.label,
                dark: theme.dark,
                path: theme.path,
            });
        }
    }

    for debugger in manifest.debuggers() {
        let program_ok = debugger
            .program
            .as_deref()
            .is_none_or(|path| safe_relative(path) && asset_exists(path));
        broken |= !program_ok;
        capabilities.push(CapabilityReport {
            name: format!("debugger {}", debugger.type_name),
            support: if program_ok {
                CapabilitySupport::NeedsAdaptation
            } else {
                CapabilitySupport::Unsupported
            },
            detail: if program_ok {
                "metadata is reusable; running the adapter requires explicit configuration and consent"
                    .into()
            } else {
                "the referenced debug adapter is missing or unsafe".into()
            },
        });
        if program_ok {
            assets.push(ExtensionAsset::Debugger {
                type_name: debugger.type_name,
                label: debugger.label,
                program: debugger.program,
                runtime: debugger.runtime,
            });
        }
    }

    let handled: BTreeSet<&str> = ["languages", "grammars", "snippets", "themes", "debuggers"]
        .into_iter()
        .collect();
    for key in manifest.contribution_keys() {
        if handled.contains(key.as_str()) {
            continue;
        }
        let (support, detail) = match key.as_str() {
            "configuration" | "configurationDefaults" | "jsonValidation" => (
                CapabilitySupport::NeedsAdaptation,
                "configuration schema can be mapped only by an explicit TermLoom adapter",
            ),
            "commands" | "keybindings" | "menus" => (
                CapabilitySupport::Unsupported,
                "VS Code commands and UI bindings are not executed",
            ),
            "views" | "viewsContainers" | "webviewPanel" | "customEditors" => (
                CapabilitySupport::Unsupported,
                "VS Code views and webviews require the Extension Host and are unsupported",
            ),
            _ => (
                CapabilitySupport::Unsupported,
                "this VS Code contribution has no safe TermLoom adapter",
            ),
        };
        capabilities.push(CapabilityReport {
            name: format!("contributes.{key}"),
            support,
            detail: detail.into(),
        });
    }

    if manifest.declares_language_server() {
        capabilities.push(CapabilityReport {
            name: "language server".into(),
            support: CapabilitySupport::NeedsAdaptation,
            detail: "the package appears to contain language tooling; configure an explicit LSP command before execution".into(),
        });
    }
    if manifest.has_entrypoint() {
        capabilities.push(CapabilityReport {
            name: "extension entrypoint".into(),
            support: CapabilitySupport::Unsupported,
            detail: "main/browser JavaScript is never executed by TermLoom".into(),
        });
    }

    let class = if broken {
        CompatibilityClass::Broken
    } else {
        CompatibilityReport::classify(&capabilities)
    };
    CompatibilityReport {
        id: manifest.id(),
        display_name: manifest.display_name.clone(),
        version: manifest.version.clone(),
        publisher: manifest.publisher.clone(),
        class,
        capabilities,
        assets,
        install_path,
    }
}

fn asset_capability(
    name: String,
    valid: bool,
    support: CapabilitySupport,
    supported_detail: &str,
) -> CapabilityReport {
    CapabilityReport {
        name,
        support: if valid {
            support
        } else {
            CapabilitySupport::Unsupported
        },
        detail: if valid {
            supported_detail.into()
        } else {
            "the referenced asset is missing or has an unsafe path".into()
        },
    }
}

pub(crate) fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entrypoint_and_webview_are_explicitly_unsupported() {
        let manifest = ExtensionManifest::parse(
            r#"{"name":"x","publisher":"p","version":"1","main":"out.js","categories":["Programming Languages"],"contributes":{"views":{"x":[]}}}"#,
        )
        .unwrap();
        let report = classify(&manifest, None, |_| true);
        assert_eq!(report.class, CompatibilityClass::Partial);
        assert!(report.capabilities.iter().any(|capability| {
            capability.name == "extension entrypoint"
                && capability.support == CapabilitySupport::Unsupported
        }));
    }

    #[test]
    fn missing_portable_asset_marks_package_broken() {
        let manifest = ExtensionManifest::parse(
            r#"{"name":"x","version":"1","contributes":{"grammars":[{"scopeName":"source.x","path":"syntaxes/x.json"}]}}"#,
        )
        .unwrap();
        let report = classify(&manifest, None, |_| false);
        assert_eq!(report.class, CompatibilityClass::Broken);
        assert!(report.assets.is_empty());
    }

    #[test]
    fn parent_and_absolute_paths_are_unsafe() {
        assert!(safe_relative(Path::new("./themes/dark.json")));
        assert!(!safe_relative(Path::new("../outside")));
        assert!(!safe_relative(Path::new("/outside")));
    }
}
