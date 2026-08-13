use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::Visit;
use syn::{ItemMod, ItemUse, Path as RustPath, UseTree};

fn rust_sources_under(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let mut pending = vec![directory.to_owned()];

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("module directory should be readable") {
            let path = entry.expect("module entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    sources
}

fn assert_sources_do_not_depend_on(paths: &[PathBuf], forbidden_module: &str) {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let violations = paths
        .iter()
        .filter(|path| {
            let source = fs::read_to_string(path).expect("Rust source should be readable");
            let current_module = module_path(&source_root, path);
            source_depends_on(&source, forbidden_module, &current_module).unwrap_or_else(|error| {
                panic!("{} should contain valid Rust: {error}", path.display())
            })
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "lower-level modules must not depend on {forbidden_module}: {violations:?}"
    );
}

fn module_path(source_root: &Path, source: &Path) -> Vec<String> {
    let relative = source
        .strip_prefix(source_root)
        .expect("Rust source should be under src")
        .with_extension("");
    let mut segments = vec!["crate".to_owned()];
    segments.extend(
        relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );
    segments
}

fn source_depends_on(
    source: &str,
    forbidden_module: &str,
    current_module: &[String],
) -> syn::Result<bool> {
    let forbidden_segments = forbidden_module.split("::").collect::<Vec<_>>();
    let syntax = syn::parse_file(source)?;
    let mut visitor = DependencyVisitor {
        forbidden_segments: &forbidden_segments,
        current_module: current_module.to_vec(),
        found: false,
    };
    visitor.visit_file(&syntax);
    Ok(visitor.found)
}

struct DependencyVisitor<'a> {
    forbidden_segments: &'a [&'a str],
    current_module: Vec<String>,
    found: bool,
}

impl DependencyVisitor<'_> {
    fn matches_segments<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> bool {
        let Some(normalized) = self.normalize_segments(segments) else {
            return false;
        };
        let mut actual = normalized.iter().map(String::as_str);
        self.forbidden_segments
            .iter()
            .all(|expected| actual.next() == Some(*expected))
    }

    fn normalize_segments<'a>(
        &self,
        segments: impl IntoIterator<Item = &'a str>,
    ) -> Option<Vec<String>> {
        let segments = segments.into_iter().collect::<Vec<_>>();
        let Some(first) = segments.first().copied() else {
            return Some(Vec::new());
        };

        match first {
            "crate" => Some(segments.into_iter().map(str::to_owned).collect()),
            "self" => Some(
                self.current_module
                    .iter()
                    .cloned()
                    .chain(segments.into_iter().skip(1).map(str::to_owned))
                    .collect(),
            ),
            "super" => {
                let mut normalized = self.current_module.clone();
                let super_count = segments
                    .iter()
                    .take_while(|segment| **segment == "super")
                    .count();
                for _ in 0..super_count {
                    if normalized.len() == 1 {
                        return None;
                    }
                    normalized.pop();
                }
                normalized.extend(segments.into_iter().skip(super_count).map(str::to_owned));
                Some(normalized)
            }
            _ => Some(segments.into_iter().map(str::to_owned).collect()),
        }
    }

    fn visit_use_tree(&mut self, tree: &UseTree, mut prefix: Vec<String>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.visit_use_tree(&path.tree, prefix);
            }
            UseTree::Name(name) => {
                prefix.push(name.ident.to_string());
                self.found |= self.matches_segments(prefix.iter().map(String::as_str));
            }
            UseTree::Rename(rename) => {
                prefix.push(rename.ident.to_string());
                self.found |= self.matches_segments(prefix.iter().map(String::as_str));
            }
            UseTree::Glob(_) => {
                self.found |= self.matches_segments(prefix.iter().map(String::as_str));
            }
            UseTree::Group(group) => {
                for item in &group.items {
                    self.visit_use_tree(item, prefix.clone());
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for DependencyVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if node.content.is_some() {
            self.current_module.push(node.ident.to_string());
            syn::visit::visit_item_mod(self, node);
            self.current_module.pop();
        } else {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        self.visit_use_tree(&node.tree, Vec::new());
    }

    fn visit_path(&mut self, node: &'ast RustPath) {
        let segments = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        self.found |= self.matches_segments(segments.iter().map(String::as_str));
        syn::visit::visit_path(self, node);
    }
}

#[test]
fn dependency_detection_covers_grouped_imports() {
    let current_module = ["crate", "presentation", "output"].map(str::to_owned);
    for source in [
        "use crate::{application::CliError, protocol};",
        "use crate::{protocol, application::CliError};",
        "use crate::{application as app, protocol};",
        "fn example() { let _ = crate::application::run; }",
    ] {
        assert!(
            source_depends_on(source, "crate::application", &current_module)
                .expect("fixture should parse"),
            "dependency should be detected in {source}"
        );
    }
}

#[test]
fn dependency_detection_covers_relative_imports() {
    let current_module = ["crate", "presentation", "output"].map(str::to_owned);
    for source in [
        "use super::super::application::CliError;",
        "use super::{super::application::CliError};",
        "fn example() { let _ = super::super::application::run; }",
        "mod nested { use super::super::super::application::CliError; }",
    ] {
        assert!(
            source_depends_on(source, "crate::application", &current_module)
                .expect("fixture should parse"),
            "relative dependency should be detected in {source}"
        );
    }
}

#[test]
fn module_paths_follow_rust_file_layout() {
    let source_root = Path::new("/workspace/src");

    assert_eq!(
        module_path(source_root, Path::new("/workspace/src/presentation.rs")),
        ["crate", "presentation"]
    );
    assert_eq!(
        module_path(
            source_root,
            Path::new("/workspace/src/presentation/output.rs")
        ),
        ["crate", "presentation", "output"]
    );
}

#[test]
fn dependency_detection_ignores_similar_names_and_text() {
    let current_module = ["crate", "presentation", "output"].map(str::to_owned);
    for source in [
        "use crate::{application_support::CliError, protocol};",
        "use super::application::CliError;",
        "use self::application::CliError;",
        "const EXAMPLE: &str = \"crate::application::CliError\";",
        "// use crate::{application::CliError};",
    ] {
        assert!(
            !source_depends_on(source, "crate::application", &current_module)
                .expect("fixture should parse"),
            "dependency should not be detected in {source}"
        );
    }
}

#[test]
fn lower_layers_do_not_depend_on_application() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut lower_layer_sources = rust_sources_under(&source_root.join("infrastructure"));
    lower_layer_sources.extend(rust_sources_under(&source_root.join("presentation")));
    lower_layer_sources.push(source_root.join("infrastructure.rs"));
    lower_layer_sources.push(source_root.join("presentation.rs"));

    assert_sources_do_not_depend_on(&lower_layer_sources, "crate::application");
}

#[test]
fn shared_error_module_does_not_depend_on_application_or_presentation() {
    let error_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/error.rs");
    let sources = vec![error_source];

    assert_sources_do_not_depend_on(&sources, "crate::application");
    assert_sources_do_not_depend_on(&sources, "crate::presentation");
}
