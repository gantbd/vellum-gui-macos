use serde_json::Value;
use std::{fs, path::Path};

const VELLUM_REVISION: &str = "a76800cf755802498d38b381d2ccd4781e58c54d";
const RECOVERY_REVISION: &str = "920dbfccf3806378fd0274cc51fadffac405931b";
const KEYSTONE_REVISION: &str = "3486252cdbaeb2d784756076f43ef6157ed0b7f8";

#[test]
fn unpublished_gui_is_bound_to_its_engine_and_recovery_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source: Value = serde_json::from_str(
        &fs::read_to_string(root.join("release/source.json")).expect("source contract"),
    )
    .expect("source JSON");
    assert_eq!(source["package"], "vellum-gui-macos");
    assert_eq!(source["vellumRevision"], VELLUM_REVISION);
    assert_eq!(source["recoveredFromVellumRevision"], RECOVERY_REVISION);
    assert_eq!(source["keystoneRevision"], KEYSTONE_REVISION);
    for field in [
        "remoteReachabilityQualified",
        "cratesIoPublished",
        "signed",
        "notarized",
    ] {
        assert_eq!(source[field], false);
    }

    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("manifest");
    assert!(manifest.contains("repository = \"https://github.com/gantbd/vellum-gui-macos\""));
    assert!(manifest.contains("publish = false"));
    assert!(manifest.contains("path = \"../vellum/host/vellum-editor\""));
    for patch in ["iced_core", "iced_graphics", "iced_widget", "objc2-app-kit"] {
        assert!(manifest.contains(&format!("{patch} = {{ path = \"vendor/")));
    }
    assert!(!manifest.contains("vellum-tui"));
    assert!(!manifest.contains("vellum-mcp"));
}

#[test]
fn vendored_geometry_and_pasteboard_contracts_are_present() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let widget = fs::read_to_string(root.join("vendor/iced_widget/src/text_editor.rs"))
        .expect("vendored widget");
    assert!(widget.contains("selection_geometry"));
    let core = fs::read_to_string(root.join("vendor/iced_core/src/text/editor.rs"))
        .expect("vendored core");
    assert!(core.contains("Read-only geometry using cached layout"));
    assert!(core.contains("fn selection_for("));
    let graphics = fs::read_to_string(root.join("vendor/iced_graphics/src/text/editor.rs"))
        .expect("vendored graphics");
    assert!(graphics.contains("fn geometry"));
    let appkit =
        fs::read_to_string(root.join("vendor/objc2-app-kit/src/generated/NSPasteboard.rs"))
            .expect("vendored AppKit binding");
    assert!(appkit.contains("releaseGlobally"));
    assert!(root.join("vendor/iced-LICENSE").is_file());
    assert!(root.join("vendor/objc2-LICENSE.md").is_file());
}

#[test]
fn github_ci_is_read_only_and_exactly_coordinates_sources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("CI workflow");
    assert_eq!(
        workflow
            .matches("actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683")
            .count(),
        3
    );
    assert!(workflow.contains("ref: a76800cf755802498d38b381d2ccd4781e58c54d"));
    assert!(workflow.contains("ref: 3486252cdbaeb2d784756076f43ef6157ed0b7f8"));
    assert!(workflow.contains("permissions:\n  contents: read"));
    for forbidden in [
        "upload-artifact",
        "cargo publish",
        "git push",
        "release create",
    ] {
        assert!(!workflow.contains(forbidden), "CI contains {forbidden}");
    }
}
