#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use vellum_iced::Gui;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--qualify-desktop" {
        #[cfg(feature = "desktop-qualification")]
        return vellum_iced::qualify_desktop();
        #[cfg(not(feature = "desktop-qualification"))]
        return Err("live probe requires explicit --features desktop-qualification; it opens a window with synthetic temporary documents only".into());
    }
    if args.len() == 1 && args[0] == "--help" {
        println!(
            "Source clipboard: Cmd-C/X/V and source toolbar buttons use the original-operation AppKit adapter. Retry delivery never repeats platform I/O. Binding is lazy; isolated private-pasteboard qualification passed. General clipboard and desktop qualification remain pending."
        );
        println!(
            "vellum-iced [EXISTING_FILE...]\nNative macOS reference GUI; one host across files; software renderer. Path entry opens additional files.\nSource and undo history remain in the engine. Generated newline LF; native Unicode 18 source horizontal movement/deletion, with version-bound gesture projection. Requires the qualified local ICU4C 79.1 installation; not a portable distribution bundle.\nLiteral find/current-or-all replacement controls; F3/Shift-F3 next/previous match. Select matches paints bounded secondary selections and delegates supported multi-edits to native history. Unsupported multi-selection gestures refuse; Select All exits. Search again after edits.\nCmd-S save, Cmd-Z undo, Cmd-Shift-Z redo, F4 editing lock, F6 next file. Quit/discard requires explicit consent.\nEditing lock does not change OS permissions. Shaping, word/line gestures and form input retain their separate widget profiles; desktop geometry, General clipboard, OS IME, bidi and accessibility remain unqualified."
        );
        println!(
            "The existing-file path bar supports Open, Export copy and Save As. Overwrites require explicit confirmation. Reference binding-history bound: 16 prior entries; missing-file creation is unsupported."
        );
        println!(
            "Comments pane: native session comments, separate note lock/history, explicit content/position points. Form Cmd-S applies; Cmd-Z/Shift-Z edits only the unapplied form. Source save does not persist notes; unapplied forms and present notes require explicit discard on close."
        );
        #[cfg(feature = "desktop-qualification")]
        println!(
            "--qualify-desktop opens an opt-in synthetic-document live window probe, captures only its own renderer, tests native save/undo and closes. No system input injection or clipboard workflow. Not a physical-input/IME/accessibility qualification."
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
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("vellum-iced currently requires the macOS filesystem adapter");
    std::process::exit(1);
}
