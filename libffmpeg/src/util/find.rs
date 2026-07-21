use std::path::PathBuf;

use libwhich::{is_valid_executable, which};

pub(crate) fn try_env(key: &str) -> Option<PathBuf> {
    let path = std::env::var_os(key).map(PathBuf::from)?;
    is_valid_executable(&path)
}

/// Validate a caller-supplied path, returning it only if it's runnable.
pub(crate) fn is_executable(path: &std::path::Path) -> Option<PathBuf> {
    is_valid_executable(path)
}

/// Search the process `PATH` only.
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    which(&[name])
        .inspect_err(|e| {
            tracing::error!(error = %e, "libwhich failed to search PATH");
        })
        .ok()?
        .next()
}

/// The login-shell fallback, gated on its opt-in flag.
#[cfg(unix)]
pub(crate) fn find_in_login_shell_path_if_enabled(name: &str) -> Option<PathBuf> {
    if !login_shell_path_enabled() {
        return None;
    }
    let path = find_in_login_shell_path(name)?;
    tracing::info!(
        binary = name,
        path = %path.display(),
        "resolved binary via login-shell PATH fallback"
    );
    Some(path)
}

/// Opt-in flag: when set (to anything other than empty / `0`), a binary that
/// can't be found on the current `PATH` is looked up again against the PATH
/// reported by the user's *login* shell.
///
/// GUI apps launched from Finder/Dock on macOS inherit launchd's minimal
/// `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), which omits Homebrew — so ffmpeg
/// installed at `/opt/homebrew/bin` is invisible even though every terminal
/// can see it. Consumers that ship as a bundled `.app` set this (e.g. via the
/// bundle's `Info.plist` `LSEnvironment`) so we recover the real PATH. It is a
/// *fallback* after the normal lookup fails, so a process launched from a shell
/// with a healthy PATH never pays for it.
const LOGIN_SHELL_PATH_KEY: &str = "LIBFFMPEG_USE_LOGIN_SHELL_PATH";

fn login_shell_path_enabled() -> bool {
    std::env::var_os(LOGIN_SHELL_PATH_KEY).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Search the user's login-shell `PATH` for `name`. The dirs are resolved once
/// per process and cached, since spawning a shell is relatively expensive.
#[cfg(unix)]
fn find_in_login_shell_path(name: &str) -> Option<PathBuf> {
    use std::sync::OnceLock;

    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    let dirs = DIRS.get_or_init(|| {
        login_shell_path()
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default()
    });

    dirs.iter()
        .find_map(|dir| libwhich::is_valid_executable_split(dir, name))
}

/// Ask the user's login shell what its `PATH` is.
///
/// Runs the shell with `-l` (login) so it sources the profile files where a
/// user's `PATH` is actually assembled (`.zprofile`, `.profile`, Homebrew's
/// `shellenv`, etc.). The value is wrapped in a sentinel and extracted from
/// between the markers, so any incidental output from the profile (a banner, a
/// `fortune`, …) can't corrupt the result.
#[cfg(unix)]
fn login_shell_path() -> Option<String> {
    use std::process::Command;

    const MARK: &str = "__LIBFFMPEG_PATH__";

    let shell = login_shell()?;
    let script = format!("printf '{MARK}%s{MARK}' \"$PATH\"");

    let output = Command::new(&shell)
        .arg("-lc")
        .arg(&script)
        .output()
        .inspect_err(
            |e| tracing::warn!(shell = %shell.display(), error = %e, "failed to run login shell for PATH"),
        )
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let start = stdout.find(MARK)? + MARK.len();
    let end = stdout[start..].find(MARK)? + start;
    Some(stdout[start..end].to_string())
}

/// Resolve the current user's login shell without assuming a specific one:
/// prefer `$SHELL` (present when launched from a terminal), and otherwise read
/// it from the passwd database — the OS's own record of the account's shell.
#[cfg(unix)]
fn login_shell() -> Option<PathBuf> {
    if let Some(shell) = std::env::var_os("SHELL") {
        if !shell.is_empty() {
            return Some(PathBuf::from(shell));
        }
    }

    // SAFETY: `getpwuid` returns a pointer into static storage owned by libc.
    // We only read from it before any other passwd call could overwrite it, and
    // copy the shell string out immediately.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_shell.is_null() {
            return None;
        }
        let shell = std::ffi::CStr::from_ptr((*pw).pw_shell).to_str().ok()?;
        (!shell.is_empty()).then(|| PathBuf::from(shell))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{find_in_login_shell_path, login_shell, login_shell_path};

    #[test]
    fn login_shell_resolves() {
        // Either $SHELL or the passwd DB must yield a shell on any real unix.
        let shell = login_shell().expect("a login shell");
        assert!(shell.is_absolute(), "shell path should be absolute: {shell:?}");
    }

    #[test]
    fn login_shell_path_is_extracted_cleanly() {
        // Marker extraction must survive any profile banner/noise and yield a
        // colon-list that contains at least the system bins.
        let path = login_shell_path().expect("login shell PATH");
        assert!(!path.is_empty());
        assert!(!path.contains("__LIBFFMPEG_PATH__"), "markers must be stripped: {path}");
        assert!(path.split(':').any(|d| d == "/bin" || d == "/usr/bin"));
    }

    #[test]
    fn finds_ubiquitous_binary_in_login_shell_path() {
        // `sh` is always in the login shell's PATH; this exercises the shell
        // spawn -> extract -> per-dir search plumbing end to end.
        assert!(find_in_login_shell_path("sh").is_some());
    }
}
