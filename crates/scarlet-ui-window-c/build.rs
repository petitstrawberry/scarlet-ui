fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("scarlet") {
        for arg in [
            "--hash-style=both",
            "-z",
            "now",
            "--entry=0",
            "--exclude-libs=ALL",
            "-soname",
            "libscarlet_ui_window.so",
        ] {
            println!("cargo::rustc-link-arg-cdylib={arg}");
        }
    }
}
