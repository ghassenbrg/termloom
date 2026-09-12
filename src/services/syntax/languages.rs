//! Language identification.
//!
//! The mapping is static for the languages TermLoom ships with, and can be
//! extended at runtime by `contributes.languages` entries from installed VSIX
//! packages (see [`crate::services::extensions`]).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A language known to the workbench. The string value is the LSP `languageId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LanguageId(pub String);

impl LanguageId {
    pub fn new(id: impl Into<String>) -> Self {
        LanguageId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Title-cased name for the status bar (`rust` → `Rust`).
    pub fn display_name(&self) -> String {
        match self.0.as_str() {
            "rust" => "Rust".into(),
            "dart" => "Dart".into(),
            "java" => "Java".into(),
            "javascript" => "JavaScript".into(),
            "javascriptreact" => "JavaScript React".into(),
            "typescript" => "TypeScript".into(),
            "typescriptreact" => "TypeScript React".into(),
            "json" => "JSON".into(),
            "jsonc" => "JSON with comments".into(),
            "yaml" => "YAML".into(),
            "toml" => "TOML".into(),
            "markdown" => "Markdown".into(),
            "shellscript" => "Shell".into(),
            "plaintext" => "Plain Text".into(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        }
    }

    /// Line-comment token, used by the toggle-comment command.
    pub fn line_comment(&self) -> Option<&'static str> {
        Some(match self.0.as_str() {
            "rust" | "dart" | "java" | "javascript" | "javascriptreact" | "typescript"
            | "typescriptreact" | "jsonc" => "//",
            "yaml" | "toml" | "shellscript" | "python" | "ruby" => "#",
            _ => return None,
        })
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Plain text: the always-available fallback.
pub fn plaintext() -> LanguageId {
    LanguageId::new("plaintext")
}

/// Built-in extension → language id table.
const EXTENSION_MAP: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("dart", "dart"),
    ("java", "java"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("jsx", "javascriptreact"),
    ("ts", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("tsx", "typescriptreact"),
    ("json", "json"),
    ("jsonc", "jsonc"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("toml", "toml"),
    ("md", "markdown"),
    ("markdown", "markdown"),
    ("sh", "shellscript"),
    ("bash", "shellscript"),
    ("zsh", "shellscript"),
    ("py", "python"),
    ("go", "go"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("hpp", "cpp"),
    ("css", "css"),
    ("html", "html"),
    ("sql", "sql"),
    ("xml", "xml"),
    ("lock", "toml"),
];

/// Built-in filename → language id table (files without a useful extension).
const FILENAME_MAP: &[(&str, &str)] = &[
    ("Cargo.lock", "toml"),
    ("Dockerfile", "dockerfile"),
    ("Makefile", "makefile"),
    (".gitignore", "ignore"),
    (".bashrc", "shellscript"),
    (".zshrc", "shellscript"),
    ("pubspec.yaml", "yaml"),
];

/// Resolves paths to languages; extendable by installed extensions.
#[derive(Debug, Clone, Default)]
pub struct LanguageRegistry {
    extra_extensions: HashMap<String, String>,
    extra_filenames: HashMap<String, String>,
}

impl LanguageRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a contribution such as `{"id":"dart","extensions":[".dart"]}`.
    pub fn register(&mut self, id: &str, extensions: &[String], filenames: &[String]) {
        for ext in extensions {
            let ext = ext.trim_start_matches('.').to_ascii_lowercase();
            if !ext.is_empty() {
                self.extra_extensions.insert(ext, id.to_string());
            }
        }
        for name in filenames {
            self.extra_filenames.insert(name.clone(), id.to_string());
        }
    }

    /// Identify the language of a path. Never fails: unknown types are
    /// `plaintext` so the editor still opens them.
    pub fn detect(&self, path: &Path) -> LanguageId {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if let Some(id) = self.extra_filenames.get(name) {
                return LanguageId::new(id.clone());
            }
            if let Some((_, id)) = FILENAME_MAP.iter().find(|(n, _)| *n == name) {
                return LanguageId::new(*id);
            }
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if let Some(id) = self.extra_extensions.get(&ext) {
                return LanguageId::new(id.clone());
            }
            if let Some((_, id)) = EXTENSION_MAP.iter().find(|(e, _)| *e == ext) {
                return LanguageId::new(*id);
            }
        }
        plaintext()
    }
}

/// Convenience wrapper using only the built-in tables.
pub fn detect(path: &Path) -> LanguageId {
    LanguageRegistry::new().detect(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_the_required_v1_languages() {
        let cases = [
            ("main.rs", "rust"),
            ("main.dart", "dart"),
            ("App.java", "java"),
            ("index.js", "javascript"),
            ("index.ts", "typescript"),
            ("package.json", "json"),
            ("pubspec.yaml", "yaml"),
            ("Cargo.toml", "toml"),
            ("README.md", "markdown"),
            ("setup.sh", "shellscript"),
        ];
        for (file, expected) in cases {
            assert_eq!(
                detect(&PathBuf::from(file)).as_str(),
                expected,
                "detecting {file}"
            );
        }
    }

    #[test]
    fn unknown_extensions_fall_back_to_plaintext() {
        assert_eq!(detect(&PathBuf::from("notes.xyz")), plaintext());
        assert_eq!(detect(&PathBuf::from("LICENSE")), plaintext());
    }

    #[test]
    fn extensions_can_add_languages() {
        let mut registry = LanguageRegistry::new();
        registry.register("kotlin", &[".kt".into(), "kts".into()], &[]);
        assert_eq!(
            registry.detect(&PathBuf::from("Main.kt")).as_str(),
            "kotlin"
        );
        assert_eq!(
            registry.detect(&PathBuf::from("build.kts")).as_str(),
            "kotlin"
        );
    }

    #[test]
    fn filename_rules_beat_extension_rules() {
        assert_eq!(detect(&PathBuf::from("Cargo.lock")).as_str(), "toml");
    }

    #[test]
    fn comment_tokens_are_known_for_common_languages() {
        assert_eq!(LanguageId::new("rust").line_comment(), Some("//"));
        assert_eq!(LanguageId::new("yaml").line_comment(), Some("#"));
        assert_eq!(LanguageId::new("json").line_comment(), None);
    }
}
