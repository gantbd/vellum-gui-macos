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
pub use gui::Gui;

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
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "vellum-desktop [EXISTING_FILE...]\nNative macOS reference GUI; source/history authority remains in the Vellum engine. Requires the qualified local ICU4C 79.1 profile; portable Unicode packaging is still pending."
        );
        return Ok(());
    }
    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    iced::application(move || Gui::open(&paths), Gui::update, Gui::view)
        .title(Gui::title)
        .subscription(Gui::subscription)
        .exit_on_close_request(false)
        .window_size((960.0, 720.0))
        .run()?;
    Ok(())
}
