fn main() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=app.manifest.xml");
    // Track payload so Cargo re-embeds when the exe is updated
    println!("cargo:rerun-if-changed=payload/NovelGenerateAgent.exe");
    let payload = PathBuf::from("payload/NovelGenerateAgent.exe");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let payload_path = if payload.is_file() {
        println!("cargo:rustc-env=NGT_PAYLOAD_IS_STUB=0");
        std::fs::canonicalize(&payload).expect("failed to resolve installer payload")
    } else {
        if profile == "release" {
            panic!(
                "release installer requires payload/NovelGenerateAgent.exe; run the payload copy step first"
            );
        }
        let stub = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR missing"))
            .join("missing-payload.stub");
        std::fs::write(&stub, b"NGT_PAYLOAD_MISSING")
            .expect("failed to write development payload stub");
        println!("cargo:warning=installer payload missing; install command will be disabled");
        println!("cargo:rustc-env=NGT_PAYLOAD_IS_STUB=1");
        stub
    };
    println!(
        "cargo:rustc-env=NGT_PAYLOAD_PATH={}",
        payload_path.display()
    );
    let windows = tauri_build::WindowsAttributes::new()
        .window_icon_path("icons/icon.ico")
        .app_manifest(include_str!("app.manifest.xml"));
    let attrs = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attrs).expect("failed to run Tauri build script");
}
