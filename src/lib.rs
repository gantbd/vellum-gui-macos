//! Iced presentation adapter over the existing editor host; no source authority.
#![forbid(unsafe_code)]
#[cfg(target_os = "macos")]
mod attachment;
#[cfg(target_os = "macos")]
mod bridge;
#[cfg(target_os = "macos")]
pub use bridge::*;
#[cfg(target_os = "macos")]
mod comment_draft;
#[cfg(target_os = "macos")]
mod gui;
#[cfg(target_os = "macos")]
pub use gui::{Gui, Message as GuiMessage};
#[cfg(target_os = "macos")]
mod service;
#[cfg(target_os = "macos")]
pub use service::*;

#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

/// Content-safe closed failures for bounded file/directory root planning.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootAdmissionError {
    Empty,
    Plan(vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1),
}

#[cfg(target_os = "macos")]
impl std::fmt::Display for RootAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "workspace root admission refused: {self:?}")
    }
}

#[cfg(target_os = "macos")]
impl std::error::Error for RootAdmissionError {}

/// Deterministically expands regular-file and directory roots without opening
/// an editor or publishing partial UI state. The editor independently
/// revalidates each resulting locator when it attaches the document.
#[cfg(target_os = "macos")]
pub fn plan_workspace_roots(
    roots: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<Vec<PathBuf>, RootAdmissionError> {
    use vellum_adapter_local_fs::{
        DirectoryDiscoveryLimits, WorkspaceRootPlanLimitsV1, WorkspaceRootPlanOutcomeV1,
    };
    let limits = WorkspaceRootPlanLimitsV1 {
        max_roots: 16,
        discovery: DirectoryDiscoveryLimits {
            max_entries: 4096,
            max_files: 16,
            max_depth: 64,
            max_path_bytes: 4096,
            max_total_path_bytes: 65_536,
        },
    };
    match vellum_adapter_local_fs::plan_workspace_roots_v1(roots, limits) {
        WorkspaceRootPlanOutcomeV1::Complete { files, .. } if files.is_empty() => {
            Err(RootAdmissionError::Empty)
        }
        WorkspaceRootPlanOutcomeV1::Complete { files, .. } => Ok(files),
        WorkspaceRootPlanOutcomeV1::Refused { reason, .. } => Err(RootAdmissionError::Plan(reason)),
    }
}

#[cfg(target_os = "macos")]
pub fn reference_session_config() -> vellum_model::ReferenceSessionConfig {
    reference_session_config_with_filesystem_limits(vellum_adapter_local_fs::FilesystemLimits {
        max_resources: 16,
        max_bytes: 65536,
        max_stage_attempts: 16,
        max_metadata_bytes: 65536,
    })
}

#[cfg(target_os = "macos")]
pub fn reference_session_config_with_filesystem_limits(
    limits: vellum_adapter_local_fs::FilesystemLimits,
) -> vellum_model::ReferenceSessionConfig {
    use vellum_adapter_local_fs::LocalFilesystemFactory;
    let mut config = vellum_model::reference_profile(
        LocalFilesystemFactory::new(limits).erase(),
        Box::new(vellum_adapter_annotation_json::JsonAnnotationCodec),
    );
    config.engine.invocation = Some(vellum_engine::coordinator::InvocationLimits {
        max_lifetime_issued: 1_000_000,
        max_retained: 8,
        max_intent_bytes: 262144,
    });
    config
}
#[cfg(all(target_os = "macos", feature = "desktop-qualification"))]
pub use gui::qualification::run as qualify_desktop;

/// Runs the qualified desktop composition with already-tokenized arguments.
#[cfg(target_os = "macos")]
pub fn run(args: Vec<std::ffi::OsString>) -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;

    if args.len() == 1 && args[0] == "--qualify-desktop" {
        #[cfg(feature = "desktop-qualification")]
        return qualify_desktop();
        #[cfg(not(feature = "desktop-qualification"))]
        return Err("live probe requires the desktop-qualification feature; it opens a window with synthetic temporary documents only".into());
    }
    if args.first().is_some_and(|argument| argument == "--service") {
        return run_service_mode(&args);
    }
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "vellum-gui-macos [EXISTING_FILE_OR_DIRECTORY...]\nvellum-gui-macos --service SOCKET [--credential CREDENTIAL]\nWith no arguments, starts empty. Service mode projects the shared workspace client without taking source/history authority. Native macOS reference GUI; requires the qualified local ICU4C 79.1 profile for embedded mode; portable Unicode packaging is still pending."
        );
        return Ok(());
    }
    let roots: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    let paths = if roots.is_empty() {
        Vec::new()
    } else {
        plan_workspace_roots(&roots)?
    };
    iced::application(move || Gui::open(&paths), Gui::update, Gui::view)
        .title(Gui::title)
        .subscription(Gui::subscription)
        .exit_on_close_request(false)
        .window_size((960.0, 720.0))
        .run()?;
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod root_admission_tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::symlink,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vellum-gui-root-admission-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn empty_roots_do_not_manufacture_a_document() {
        assert_eq!(
            plan_workspace_roots(std::iter::empty::<&Path>()),
            Err(RootAdmissionError::Empty)
        );
        let fixture = Fixture::new();
        assert_eq!(
            plan_workspace_roots([&fixture.0]),
            Err(RootAdmissionError::Empty)
        );
    }

    #[test]
    fn plans_are_sorted_duplicate_free_and_links_refuse_completely() {
        let fixture = Fixture::new();
        let nested = fixture.0.join("nested");
        fs::create_dir(&nested).unwrap();
        let first = fixture.0.join("b.txt");
        let second = nested.join("a.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        let plan = plan_workspace_roots([fixture.0.as_path(), first.as_path()]).unwrap();
        let mut expected = vec![first.clone(), second];
        expected.sort();
        assert_eq!(plan, expected);

        symlink(&first, fixture.0.join("link.txt")).unwrap();
        assert!(matches!(
            plan_workspace_roots([&fixture.0]),
            Err(RootAdmissionError::Plan(
                vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1::Discovery(
                    vellum_adapter_local_fs::DirectoryDiscoveryRefusal::UnsupportedResource
                )
            ))
        ));
    }

    #[test]
    fn aggregate_plan_refuses_more_than_the_editor_capacity() {
        let fixture = Fixture::new();
        let mut roots = Vec::new();
        for index in 0..17 {
            let path = fixture.0.join(format!("{index:02}.txt"));
            fs::write(&path, index.to_string()).unwrap();
            roots.push(path);
        }
        assert_eq!(
            plan_workspace_roots(&roots),
            Err(RootAdmissionError::Plan(
                vellum_adapter_local_fs::WorkspaceRootPlanRefusalV1::RootLimit
            ))
        );
    }
}
