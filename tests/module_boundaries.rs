use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

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

fn assert_sources_do_not_reference_layer(paths: &[PathBuf], forbidden_layer: &str) {
    let violations = paths
        .iter()
        .filter(|path| {
            let source = fs::read_to_string(path).expect("Rust source should be readable");
            source_mentions_identifier(&source, forbidden_layer).unwrap_or_else(|error| {
                panic!(
                    "{} should contain valid Rust tokens: {error}",
                    path.display()
                )
            })
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "lower-level sources must not reference the {forbidden_layer} layer: {violations:?}"
    );
}

fn assert_sources_do_not_use_external_source_inclusion(paths: &[PathBuf]) {
    let violations = paths
        .iter()
        .filter(|path| {
            let source = fs::read_to_string(path).expect("Rust source should be readable");
            let tokens = source.parse::<TokenStream>().unwrap_or_else(|error| {
                panic!(
                    "{} should contain valid Rust tokens: {error}",
                    path.display()
                )
            });
            token_stream_includes_external_rust(tokens)
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "layer-checked sources must not include Rust from unscanned paths: {violations:?}"
    );
}

fn token_stream_includes_external_rust(tokens: TokenStream) -> bool {
    let tokens = tokens.into_iter().collect::<Vec<_>>();

    for (index, token) in tokens.iter().enumerate() {
        if token_is_identifier(token, "include") {
            return true;
        }

        if matches!(token, TokenTree::Punct(punctuation) if punctuation.as_char() == '#')
            && let Some(TokenTree::Group(group)) = tokens.get(index + 1)
            && group.delimiter() == Delimiter::Bracket
            && token_stream_contains_path_assignment(group.stream())
        {
            return true;
        }

        if let TokenTree::Group(group) = token
            && token_stream_includes_external_rust(group.stream())
        {
            return true;
        }
    }

    false
}

fn token_stream_contains_path_assignment(tokens: TokenStream) -> bool {
    let tokens = tokens.into_iter().collect::<Vec<_>>();

    for (index, token) in tokens.iter().enumerate() {
        if token_is_identifier(token, "path")
            && matches!(tokens.get(index + 1), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '=')
        {
            return true;
        }

        if let TokenTree::Group(group) = token
            && token_stream_contains_path_assignment(group.stream())
        {
            return true;
        }
    }

    false
}

fn token_is_identifier(token: &TokenTree, expected: &str) -> bool {
    let TokenTree::Ident(identifier) = token else {
        return false;
    };
    let identifier = identifier.to_string();
    identifier.strip_prefix("r#").unwrap_or(&identifier) == expected
}

/// Layer names are reserved identifiers in lower-level source files.
///
/// This deliberately checks tokens instead of reproducing Rust name
/// resolution. Any code token naming an upper layer is rejected, so grouped
/// imports, relative paths, aliases, block scopes, and macro input cannot
/// bypass the boundary. Comments and literals are not identifier tokens.
fn source_mentions_identifier(
    source: &str,
    forbidden_identifier: &str,
) -> Result<bool, proc_macro2::LexError> {
    let tokens = source.parse::<TokenStream>()?;
    Ok(token_stream_mentions_identifier(
        tokens,
        forbidden_identifier,
    ))
}

fn token_stream_mentions_identifier(tokens: TokenStream, forbidden_identifier: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        TokenTree::Ident(_) => token_is_identifier(&token, forbidden_identifier),
        TokenTree::Group(group) => {
            token_stream_mentions_identifier(group.stream(), forbidden_identifier)
        }
        TokenTree::Punct(_) | TokenTree::Literal(_) => false,
    })
}

#[test]
fn identifier_detection_covers_dependency_spellings() {
    for source in [
        "use crate::application::CliError;",
        "use crate::{application::CliError, protocol};",
        "use super::super::application::CliError;",
        "use crate as root; use root::application::CliError;",
        "use crate::{self as root}; use root::application::CliError;",
        "use super::super as root; use root::application::CliError;",
        "use super::{super as root}; use root::application::CliError;",
        "use crate as root; use root as root2; use root2::application::CliError;",
        "extern crate self as root; use root::application::CliError;",
        "fn example() { use crate as root; let _ = root::application::run; }",
        "use crate as root; fn example() { let _ = self::root::application::run; }",
    ] {
        assert!(
            source_mentions_identifier(source, "application").expect("fixture should lex"),
            "layer identifier should be detected in {source}"
        );
    }
}

#[test]
fn identifier_detection_covers_macro_tokens_and_raw_identifiers() {
    for source in [
        "delegate!(crate::application::run());",
        "macro_rules! delegate { () => { crate::application::run() } }",
        "#[derive(crate::application::Example)] struct Example;",
        "use crate::r#application::CliError;",
    ] {
        assert!(
            source_mentions_identifier(source, "application").expect("fixture should lex"),
            "layer identifier should be detected in {source}"
        );
    }
}

#[test]
fn identifier_detection_ignores_comments_literals_and_similar_identifiers() {
    for source in [
        "// use crate::application::CliError;",
        "/* use crate::application::CliError; */",
        "const EXAMPLE: &str = \"crate::application::CliError\";",
        "const EXAMPLE: &str = r#\"crate::application::CliError\"#;",
        "use crate::application_support::CliError;",
        "let application_count = 1;",
    ] {
        assert!(
            !source_mentions_identifier(source, "application").expect("fixture should lex"),
            "layer identifier should not be detected in {source}"
        );
    }
}

#[test]
fn layer_names_are_intentionally_reserved_as_identifiers() {
    for source in [
        "let application = 1;",
        "#[cfg(application)] fn example() {}",
    ] {
        assert!(
            source_mentions_identifier(source, "application").expect("fixture should lex"),
            "reserved layer identifier should be detected in {source}"
        );
    }
}

#[test]
fn external_rust_inclusion_detection_is_token_aware() {
    for source in [
        "include!(\"generated.rs\");",
        "r#include!(\"generated.rs\");",
        "use std::include as load; load!(\"generated.rs\");",
        "mod nested { include!(\"generated.rs\"); }",
        "#[path = \"generated.rs\"] mod generated;",
        "#[r#path = \"generated.rs\"] mod generated;",
        "#[cfg_attr(all(), path = \"generated.rs\")] mod generated;",
        "#[r#cfg_attr(all(), r#path = \"generated.rs\")] mod generated;",
    ] {
        let tokens = source.parse::<TokenStream>().expect("fixture should lex");
        assert!(
            token_stream_includes_external_rust(tokens),
            "external Rust inclusion should be detected in {source}"
        );
    }

    for source in [
        "include_str!(\"fixture.txt\");",
        "use std::path::Path;",
        "const EXAMPLE: &str = \"include!(generated.rs) #[path]\";",
    ] {
        let tokens = source.parse::<TokenStream>().expect("fixture should lex");
        assert!(
            !token_stream_includes_external_rust(tokens),
            "external Rust inclusion should not be detected in {source}"
        );
    }
}

#[test]
fn library_crate_contains_only_lower_layers() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let source = fs::read_to_string(source_root.join("lib.rs")).expect("lib.rs should be readable");
    let tokens = source.parse::<TokenStream>().expect("lib.rs should lex");

    assert_eq!(
        tokens.to_string(),
        "pub mod cli ; pub mod domain ; pub mod infrastructure ; pub mod protocol ;"
    );
}

#[test]
fn lower_library_layers_follow_the_dependency_order() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let protocol_sources = vec![source_root.join("protocol.rs")];
    let domain_sources = vec![source_root.join("domain.rs")];
    let mut cli_sources = rust_sources_under(&source_root.join("cli"));
    cli_sources.push(source_root.join("cli.rs"));
    let mut infrastructure_sources = rust_sources_under(&source_root.join("infrastructure"));
    infrastructure_sources.push(source_root.join("infrastructure.rs"));

    for forbidden in ["cli", "domain", "infrastructure"] {
        assert_sources_do_not_reference_layer(&protocol_sources, forbidden);
    }
    for forbidden in ["cli", "infrastructure"] {
        assert_sources_do_not_reference_layer(&domain_sources, forbidden);
    }
    for forbidden in ["domain", "infrastructure", "protocol"] {
        assert_sources_do_not_reference_layer(&cli_sources, forbidden);
    }
    assert_sources_do_not_reference_layer(&infrastructure_sources, "cli");

    let all_sources = protocol_sources
        .into_iter()
        .chain(domain_sources)
        .chain(cli_sources)
        .chain(infrastructure_sources)
        .collect::<Vec<_>>();
    assert_sources_do_not_use_external_source_inclusion(&all_sources);
}

#[test]
fn binary_layers_follow_the_dependency_order() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let error_sources = vec![source_root.join("error.rs")];
    let mut presentation_sources = rust_sources_under(&source_root.join("presentation"));
    presentation_sources.push(source_root.join("presentation.rs"));

    for forbidden in ["application", "presentation"] {
        assert_sources_do_not_reference_layer(&error_sources, forbidden);
    }
    assert_sources_do_not_reference_layer(&presentation_sources, "application");

    let checked_sources = error_sources
        .into_iter()
        .chain(presentation_sources)
        .collect::<Vec<_>>();
    assert_sources_do_not_use_external_source_inclusion(&checked_sources);
}
