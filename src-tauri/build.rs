fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/native/content_vision.m");
        cc::Build::new()
            .file("src/native/content_vision.m")
            .flag(format!(
                "-mmacosx-version-min={}",
                std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "10.13".into())
            ))
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .compile("lumetrace_content_vision");
        for framework in ["Foundation", "Vision", "PDFKit", "ImageIO", "CoreGraphics"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    tauri_build::build()
}
