fn main() {
    slint_build::compile("ui/app.slint").expect("failed to compile ui/app.slint");
    embed_icon();
}

/// The executable's icon lives in a Windows resource, not in the Slint markup:
/// that is what the taskbar, Explorer and the title bar all read, and it lets
/// Windows pick the hand-tuned 16 px drawing rather than downscaling a large one.
///
/// Embedding needs a resource compiler from the Windows SDK. It is cosmetic, so a
/// machine without one gets a warning and a working build rather than a failure.
#[cfg(windows)]
fn embed_icon() {
    println!("cargo:rerun-if-changed=assets/tinyget.ico");
    if let Err(e) = winresource::WindowsResource::new()
        .set_icon("assets/tinyget.ico")
        .compile()
    {
        println!("cargo:warning=could not embed the application icon: {e}");
    }
}

#[cfg(not(windows))]
fn embed_icon() {}
