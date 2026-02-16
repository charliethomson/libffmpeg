use std::path::PathBuf;

use libwhich::{is_valid_executable, which};

fn try_env(key: &str) -> Option<PathBuf> {
    let path = std::env::var_os(key).map(PathBuf::from)?;
    is_valid_executable(&path)
}

pub fn find_binary(name: &str, env_key: &str) -> Option<PathBuf> {
    if let Some(path) = try_env(env_key) {
        return Some(path);
    }

    which(&[name])
        .inspect_err(|e| {
            tracing::error!(
                error = %e,
                "libwhich failed to find ffmpeg binary"
            );
        })
        .ok()?
        .next()
}
