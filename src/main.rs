#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    vellum_gui_macos::run(args)
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("vellum-gui-macos currently requires the macOS filesystem adapter");
    std::process::exit(1);
}
