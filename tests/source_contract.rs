use serde_json::Value;
use std::{fs, path::Path};

const VELLUM_REVISION: &str = "f4a53ba664acf2a12b6fadfd7d378eef63307c39";
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
    assert!(manifest.contains("vellum-sdk-client"));
    for forbidden in ["vellum-client =", "vellum-transport =", "vellum-protocol ="] {
        assert!(
            !manifest.contains(forbidden),
            "GUI peer bypasses the SDK through {forbidden}"
        );
    }
    assert!(manifest.contains("package = \"plushie-iced\""));
    assert!(manifest.contains("version = \"=0.8.4\""));
    assert!(manifest.contains("\"a11y\""));
    for patch in [
        "plushie-iced-core",
        "plushie-iced-graphics",
        "plushie-iced-widget",
        "plushie-iced-futures",
        "plushie-iced-winit",
        "objc2-app-kit",
    ] {
        assert!(manifest.contains(&format!("{patch} = {{ path = \"vendor/")));
    }
    for retired in ["iced_core", "iced_graphics", "iced_widget"] {
        assert!(
            !manifest.contains(&format!("{retired} = {{ path = \"vendor/")),
            "retired stock-Iced patch remains active: {retired}"
        );
    }
    assert!(!manifest.contains("vellum-tui"));
    assert!(!manifest.contains("vellum-mcp"));

    let readme = fs::read_to_string(root.join("README.md")).expect("README");
    assert!(readme.contains("Cargo applies `[patch]` only at the workspace root"));
    assert!(readme.contains("must select Vellum's qualified vendored Plushie packages"));
    assert!(readme.contains("Component close never calls"));

    let facade = fs::read_to_string(root.join("src/lib.rs")).expect("library facade");
    for port in [
        "SharedWorkspaceHost",
        "WorkspaceAdmissionError",
        "WorkspaceHost",
    ] {
        assert!(facade.contains(port), "library facade omits {port}");
    }
    let presentation = fs::read_to_string(root.join("src/gui.rs")).expect("GUI presentation");
    assert!(presentation.contains("pub fn open(host: SharedWorkspaceHost"));
    assert!(!presentation.contains("Bridge::open_reference"));
    assert!(!presentation.contains("vellum_adapter_local_fs"));
}

#[test]
fn vendored_geometry_and_pasteboard_contracts_are_present() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let widget = fs::read_to_string(root.join("vendor/plushie-iced-widget/src/text_editor.rs"))
        .expect("vendored widget");
    assert!(widget.contains("selection_geometry"));
    let core = fs::read_to_string(root.join("vendor/plushie-iced-core/src/text/editor.rs"))
        .expect("vendored core");
    assert!(core.contains("fn selection_for("));
    let graphics = fs::read_to_string(root.join("vendor/plushie-iced-graphics/src/text/editor.rs"))
        .expect("vendored graphics");
    assert!(graphics.contains("fn selection_for("));
    let winit = fs::read_to_string(root.join("vendor/plushie-iced-winit/Cargo.toml"))
        .expect("vendored window runtime");
    assert!(winit.contains("[target.'cfg(target_arch = \"wasm32\")'.dependencies.web-sys]"));
    assert!(winit.contains("version = \"=0.3.107\""));
    assert!(root.join("vendor/plushie-iced-futures").is_dir());
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
    assert!(workflow.contains("ref: f4a53ba664acf2a12b6fadfd7d378eef63307c39"));
    assert!(workflow.contains("ref: 3486252cdbaeb2d784756076f43ef6157ed0b7f8"));
    assert!(workflow.contains("permissions:\n  contents: read"));
    assert!(workflow.contains("./tests/live/service_journey.sh"));
    for forbidden in [
        "upload-artifact",
        "cargo publish",
        "git push",
        "release create",
    ] {
        assert!(!workflow.contains(forbidden), "CI contains {forbidden}");
    }
}

#[test]
fn live_service_journey_uses_only_the_peer_sdk_and_real_daemon_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let journey = fs::read_to_string(root.join("tests/live/service_journey.sh"))
        .expect("live service journey");
    assert!(journey.contains("--package vellum-daemon"));
    assert!(journey.contains("--allow-edit --allow-save"));
    assert!(journey.contains("--test live_service"));
    assert!(journey.contains("--shutdown"));
    assert!(!journey.contains("cargo publish"));
    assert!(!journey.contains("git push"));

    let test = fs::read_to_string(root.join("tests/live_service.rs")).expect("live test");
    assert!(test.contains("vellum_sdk_client"));
    assert!(test.contains("ServiceGui"));
    assert!(!test.contains("vellum_daemon"));
}
