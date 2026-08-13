use std::fs;
use std::path::{Path, PathBuf};

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
    let violations = paths
        .iter()
        .filter(|path| {
            fs::read_to_string(path)
                .expect("Rust source should be readable")
                .contains(forbidden_module)
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "lower-level modules must not depend on {forbidden_module}: {violations:?}"
    );
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
