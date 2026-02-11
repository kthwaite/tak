#[test]
fn main_stays_thin_wrapper_over_core_cli() {
    let source = include_str!("../src/main.rs");

    assert!(source.contains("tak_core::cli::run_cli()"));
    assert!(source.contains("std::process::exit"));

    // Architectural guardrail: tak-cli should remain a tiny process wrapper,
    // not a second home for parser/dispatch/business logic.
    assert!(
        !source.contains("clap::"),
        "tak-cli main.rs should not import clap directly"
    );
    assert!(
        !source.contains("match cli.command"),
        "tak-cli main.rs should not implement command dispatch"
    );

    let non_empty_non_comment_lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .count();

    assert!(
        non_empty_non_comment_lines <= 4,
        "tak-cli main.rs should remain tiny (found {non_empty_non_comment_lines} significant lines)"
    );
}

#[test]
fn manifest_avoids_direct_cli_framework_dependencies() {
    let manifest = include_str!("../Cargo.toml");

    assert!(manifest.contains("tak-core"));
    assert!(
        !manifest.contains("clap"),
        "tak-cli should not depend on clap directly; parser/dispatch live in tak-core"
    );
}
