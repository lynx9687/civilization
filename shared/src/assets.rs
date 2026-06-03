use std::path::PathBuf;

/// Environment variable used to override where runtime assets are loaded from.
pub const ASSET_DIR_ENV: &str = "CIVILIZATION_ASSET_DIR";

/// Finds the runtime asset directory for both packaged releases and local development.
#[cfg(not(target_arch = "wasm32"))]
pub fn assets_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(ASSET_DIR_ENV) {
        return PathBuf::from(path);
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(exe_dir) = current_exe.parent()
    {
        let path = exe_dir.join("assets");
        if path.is_dir() {
            return path;
        }
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let path = current_dir.join("assets");
        if path.is_dir() {
            return path;
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets")
}

/// Assets dir for web
#[cfg(target_arch = "wasm32")]
pub fn assets_dir() -> PathBuf {
    PathBuf::from("./assets")
}