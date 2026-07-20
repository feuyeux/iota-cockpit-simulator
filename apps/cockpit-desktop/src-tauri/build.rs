fn main() {
    // Tauri validates every externalBin entry even for `cargo test`. The real
    // binaries are staged by the tauri npm scripts; keep plain workspace
    // builds self-contained with disposable placeholders when they are absent.
    let target = std::env::var("TARGET").expect("Cargo always provides TARGET");
    let executable_extension = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    for binary in ["cockpit-simulator", "cockpit-evaluator"] {
        let sidecar = std::path::Path::new("binaries")
            .join(format!("{binary}-{target}{executable_extension}"));
        if !sidecar.exists() {
            std::fs::create_dir_all(sidecar.parent().expect("sidecar has a parent"))
                .expect("create sidecar directory");
            std::fs::write(&sidecar, []).expect("create sidecar placeholder");
        }
    }
    tauri_build::build();
}
