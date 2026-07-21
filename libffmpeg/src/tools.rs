//! Locating the ffmpeg suite on the host.
//!
//! Historically the only way to point libffmpeg at a specific binary was an
//! environment variable, which forces a host application that stores the path
//! in its own config to call [`std::env::set_var`] — `unsafe` on edition 2024
//! and unsound once any thread exists. [`set_tool_path`] is the supported
//! replacement: it takes precedence over the environment and can be called at
//! any point in a process's life.
//!
//! [`locate`] exposes the whole resolution, including *how* a binary was found,
//! so a host can report its media-tool status to the user rather than
//! discovering the problem at spawn time.

use std::path::PathBuf;
use std::sync::RwLock;

use crate::util::find::{find_on_path, is_executable, try_env};

/// A binary from the ffmpeg suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
    /// The bundled player. libffmpeg never spawns it, but hosts that do (a
    /// "watch this stream" feature) want the same discovery and reporting.
    Ffplay,
}

impl Tool {
    /// Every tool, in the order a status report should list them.
    pub const ALL: [Self; 3] = [Self::Ffmpeg, Self::Ffprobe, Self::Ffplay];

    /// The binary's name, without any platform extension.
    #[must_use]
    pub const fn binary_name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Ffplay => "ffplay",
        }
    }

    /// The environment variable that overrides this tool's location.
    #[must_use]
    pub const fn env_key(self) -> &'static str {
        match self {
            Self::Ffmpeg => "LIBFFMPEG_FFMPEG_PATH",
            Self::Ffprobe => "LIBFFMPEG_FFPROBE_PATH",
            Self::Ffplay => "LIBFFMPEG_FFPLAY_PATH",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Ffmpeg => 0,
            Self::Ffprobe => 1,
            Self::Ffplay => 2,
        }
    }
}

/// Where a resolved binary came from. Ordered as it is tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSource {
    /// Set by the host application via [`set_tool_path`].
    Configured,
    /// The tool's `LIBFFMPEG_*_PATH` environment variable.
    Environment,
    /// Found on the process's `PATH`.
    Path,
    /// Found in a well-known install location for this platform.
    PlatformDefault,
    /// Found on the *login* shell's `PATH` (see the opt-in fallback below).
    LoginShell,
}

impl ToolSource {
    /// A short human-readable label, for status output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Environment => "environment",
            Self::Path => "PATH",
            Self::PlatformDefault => "platform default",
            Self::LoginShell => "login shell PATH",
        }
    }
}

/// A resolved binary and the reason we resolved it that way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub path: PathBuf,
    pub source: ToolSource,
}

/// Host-supplied overrides, highest precedence. Indexed by [`Tool::index`].
///
/// A plain `RwLock` rather than a `OnceLock`: a host with a settings UI can
/// change the path at runtime, and the next spawn must honour it (nothing here
/// caches a resolved binary).
static OVERRIDES: RwLock<[Option<PathBuf>; 3]> = RwLock::new([None, None, None]);

/// Point a tool at a specific binary, taking precedence over the environment
/// and any search. `None` clears a previous override.
///
/// This is the safe alternative to setting the tool's environment variable: it
/// needs no `unsafe`, and unlike `set_var` it can be called after threads exist.
/// The path is *not* validated here — a host that wants to reject a bad path
/// should check [`locate`] afterwards, so that a typo surfaces in the host's own
/// diagnostics rather than as a spawn failure.
pub fn set_tool_path(tool: Tool, path: Option<PathBuf>) {
    // A poisoned lock means a previous writer panicked mid-update; the data is
    // still structurally fine (an array of Options), so recover rather than
    // propagate a panic into an unrelated caller.
    let mut guard = OVERRIDES
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard[tool.index()] = path;
}

/// The override currently set for `tool`, if any.
#[must_use]
pub fn tool_path_override(tool: Tool) -> Option<PathBuf> {
    OVERRIDES
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)[tool.index()]
        .clone()
}

/// Resolve a tool, reporting where it was found.
///
/// Precedence: host override → environment → `PATH` → platform defaults →
/// login-shell `PATH` (opt-in, unix only). Nothing is cached: a host that
/// changes the override, or a user that installs ffmpeg while the app is
/// running, takes effect on the next call.
#[must_use]
pub fn locate(tool: Tool) -> Option<Located> {
    if let Some(path) = tool_path_override(tool).and_then(|p| is_executable(&p)) {
        return Some(Located { path, source: ToolSource::Configured });
    }
    if let Some(path) = try_env(tool.env_key()) {
        return Some(Located { path, source: ToolSource::Environment });
    }
    if let Some(path) = find_on_path(tool.binary_name()) {
        return Some(Located { path, source: ToolSource::Path });
    }
    if let Some(path) = find_in_platform_defaults(tool.binary_name()) {
        return Some(Located { path, source: ToolSource::PlatformDefault });
    }
    #[cfg(unix)]
    if let Some(path) = crate::util::find::find_in_login_shell_path_if_enabled(tool.binary_name()) {
        return Some(Located { path, source: ToolSource::LoginShell });
    }
    None
}

/// Resolve a tool to a path, discarding the provenance.
#[must_use]
pub fn find(tool: Tool) -> Option<PathBuf> {
    locate(tool).map(|l| l.path)
}

/// Well-known install locations to try when a tool isn't on `PATH`.
///
/// This exists because the common failure is a *correct* install that the
/// process simply can't see: a GUI app launched from Finder or the Start Menu
/// inherits a minimal environment, and on Windows there's no login-shell trick
/// to fall back on. These are the directories the platform's usual package
/// managers install into.
#[must_use]
pub fn platform_default_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Homebrew: /opt/homebrew on Apple silicon, /usr/local on Intel.
        // MacPorts installs to /opt/local.
        vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/local/bin"),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        let mut dirs = Vec::new();
        // winget shims, then the two common manual/choco install roots.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::Path::new(&local).join("Microsoft").join("WinGet").join("Links"));
        }
        for root in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(root) {
                dirs.push(std::path::Path::new(&p).join("ffmpeg").join("bin"));
            }
        }
        if let Some(drive) = std::env::var_os("SystemDrive") {
            dirs.push(std::path::Path::new(&drive).join("\\ffmpeg\\bin"));
        }
        if let Some(programdata) = std::env::var_os("ProgramData") {
            dirs.push(std::path::Path::new(&programdata).join("chocolatey").join("bin"));
        }
        dirs
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/snap/bin"),
        ]
    }
}

fn find_in_platform_defaults(name: &str) -> Option<PathBuf> {
    platform_default_dirs()
        .iter()
        .find_map(|dir| libwhich::is_valid_executable_split(dir, name))
}

/// Suggested way to install the suite on this platform, for a host to show the
/// user when a tool is missing. Returns `(command, download_url)`; the command
/// is `None` when there's no single obvious package manager.
#[must_use]
pub fn install_hint() -> (Option<&'static str>, &'static str) {
    #[cfg(target_os = "macos")]
    {
        (Some("brew install ffmpeg"), "https://ffmpeg.org/download.html#build-mac")
    }
    #[cfg(target_os = "windows")]
    {
        // gyan.dev is the build ffmpeg.org itself links to for Windows.
        (
            Some("winget install Gyan.FFmpeg"),
            "https://www.gyan.dev/ffmpeg/builds/",
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        (None, "https://ffmpeg.org/download.html#build-linux")
    }
}

#[cfg(test)]
mod tests {
    use super::{Tool, set_tool_path, tool_path_override};
    use std::path::PathBuf;

    #[test]
    fn override_roundtrips_per_tool() {
        set_tool_path(Tool::Ffprobe, Some(PathBuf::from("/tmp/ffprobe")));
        assert_eq!(tool_path_override(Tool::Ffprobe), Some(PathBuf::from("/tmp/ffprobe")));
        // Independent slots: setting one must not disturb another.
        assert_eq!(tool_path_override(Tool::Ffplay), None);
        set_tool_path(Tool::Ffprobe, None);
        assert_eq!(tool_path_override(Tool::Ffprobe), None);
    }

    #[test]
    fn tools_have_distinct_names_and_keys() {
        let names: Vec<_> = Tool::ALL.iter().map(|t| t.binary_name()).collect();
        let keys: Vec<_> = Tool::ALL.iter().map(|t| t.env_key()).collect();
        for i in 0..Tool::ALL.len() {
            for j in (i + 1)..Tool::ALL.len() {
                assert_ne!(names[i], names[j]);
                assert_ne!(keys[i], keys[j]);
            }
        }
    }

    #[test]
    fn platform_defaults_are_absolute() {
        for dir in super::platform_default_dirs() {
            assert!(dir.is_absolute(), "{dir:?} should be absolute");
        }
    }
}
