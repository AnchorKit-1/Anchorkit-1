//! Builds the on-disk path to the release wasm artifact that
//! `testnet_integration_tests` deploys via the Stellar CLI (see
//! `wasm_path` there).
//!
//! Kept in its own module, always compiled under plain `cfg(test)` rather
//! than gated behind the `testnet-integration` feature, so its
//! path-joining logic runs on every `cargo test` -- including the Windows
//! runner in CI (`.github/workflows/ci.yml`). The gated integration test
//! itself only ever runs locally against a funded testnet identity, which
//! means a Windows-specific path bug in it would otherwise go unnoticed
//! until someone hit it by hand.

extern crate std;

use std::path::PathBuf;

/// Joins `manifest_dir` with the fixed release-wasm path segments via
/// `PathBuf`'s `FromIterator`, so the platform-native separator (`/` on
/// Unix, `\` on Windows) is used regardless of what `manifest_dir` itself
/// looks like -- a Unix-style absolute path, or a Windows-style
/// `C:\...` path on a Windows CI runner.
pub(crate) fn wasm_artifact_path(manifest_dir: &str) -> PathBuf {
    [
        manifest_dir,
        "target",
        "wasm32v1-none",
        "release",
        "anchorkit.wasm",
    ]
    .iter()
    .collect()
}

#[test]
fn joins_segments_as_separate_path_components() {
    use std::path::Component;

    let path = wasm_artifact_path("/repo");
    let normal_components: std::vec::Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    // Each segment must land as its own path component -- joined via
    // `PathBuf`, not glued together by hardcoded string formatting, which
    // is the kind of change that reads fine on Unix but silently produces
    // a broken path on Windows.
    assert_eq!(
        normal_components,
        std::vec![
            "repo",
            "target",
            "wasm32v1-none",
            "release",
            "anchorkit.wasm"
        ],
        "expected each segment to be its own path component"
    );
}

#[test]
fn preserves_a_windows_style_manifest_dir_verbatim() {
    // CARGO_MANIFEST_DIR on a Windows CI runner looks like
    // `C:\a\Anchorkit-1\Anchorkit-1`, not a Unix-style path. It must be
    // preserved as the path's leading text rather than mangled by
    // whatever separator-joining logic appends the fixed segments after it.
    let windows_style_manifest_dir = r"C:\a\Anchorkit-1\Anchorkit-1";
    let path = wasm_artifact_path(windows_style_manifest_dir);
    let joined = path.to_string_lossy().into_owned();

    assert!(
        joined.starts_with(windows_style_manifest_dir),
        "expected manifest_dir to remain the path's leading text, got {joined}"
    );
    assert!(
        joined.ends_with("anchorkit.wasm"),
        "expected anchorkit.wasm as the final path segment, got {joined}"
    );
}

#[test]
fn wasm_filename_is_the_final_component_regardless_of_manifest_dir_trailing_separator() {
    // A manifest dir with (Unix) or without a trailing separator must
    // still produce a path whose final component is exactly the wasm
    // filename, not an empty segment or a doubled separator.
    let with_trailing = wasm_artifact_path("/repo/");
    let without_trailing = wasm_artifact_path("/repo");

    assert_eq!(
        with_trailing.file_name().and_then(|n| n.to_str()),
        Some("anchorkit.wasm"),
    );
    assert_eq!(
        without_trailing.file_name().and_then(|n| n.to_str()),
        Some("anchorkit.wasm"),
    );
}
