fn main() {
    println!("cargo:rerun-if-changed=assets/media-sift-icon.ico");

    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut resources = winresource::WindowsResource::new();
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
            if let Some(windres) = preferred_gnu_tool("windres.exe", "MEDIA_SIFT_WINDRES") {
                resources.set_windres_path(&windres.to_string_lossy());
            }
            if let Some(ar) = preferred_gnu_tool("ar.exe", "MEDIA_SIFT_AR") {
                resources.set_ar_path(&ar.to_string_lossy());
            }
        }
        resources
            .set_icon("assets/media-sift-icon.ico")
            .set("ProductName", "MediaSift")
            .set("OriginalFilename", "media-sift.exe");
        resources
            .compile()
            .expect("compile MediaSift Windows icon resources");
    }
}

fn preferred_gnu_tool(name: &str, override_variable: &str) -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os(override_variable).map(std::path::PathBuf::from) {
        return Some(path);
    }

    let mut candidates = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();

    candidates.sort_by_key(|path| {
        let path = path
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if ["\\ucrt64\\", "\\mingw64\\", "\\clang64\\"]
            .iter()
            .any(|prefix| path.contains(prefix))
        {
            0
        } else if path.contains("\\usr\\bin\\") {
            2
        } else {
            1
        }
    });
    candidates.into_iter().next()
}
