use liberror::AnyError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::task::JoinSet;
use tracing::{Instrument, Span, instrument};
use valuable::Valuable;

#[derive(Debug, Clone, Serialize, Deserialize, Valuable)]
pub enum FileType {
    File,
    Symlink,
    Directory,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
    Other,
}
impl From<std::fs::FileType> for FileType {
    fn from(value: std::fs::FileType) -> Self {
        if value.is_dir() {
            return Self::Directory;
        }
        if value.is_file() {
            return Self::File;
        }
        if value.is_symlink() {
            return Self::Symlink;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;

            if value.is_block_device() {
                return Self::BlockDevice;
            }
            if value.is_char_device() {
                return Self::CharDevice;
            }
            if value.is_fifo() {
                return Self::Fifo;
            }
            if value.is_socket() {
                return Self::Socket;
            }
        }

        Self::Other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Valuable, Error)]
pub enum FindBinaryError {
    #[error("Failed to canonicalize search path '{search_path}': {inner_error}")]
    SearchPathCanonicalize {
        search_path: String,
        inner_error: AnyError,
    },
    #[error("Failed to get metadata for search path '{search_path}': {inner_error}")]
    SearchPathMetadata {
        search_path: String,
        inner_error: AnyError,
    },
    #[error("Failed to open directory '{search_path}': {inner_error}")]
    OpenReadDir {
        search_path: String,
        inner_error: AnyError,
    },
    #[error("Failed to read directory entry in '{search_path}': {inner_error}")]
    ReadDirEntry {
        search_path: String,
        inner_error: AnyError,
    },
    #[error("Failed to canonicalize binary path '{binary_path}': {inner_error}")]
    BinaryPathCanonicalize {
        binary_path: String,
        inner_error: AnyError,
    },
    #[error("Failed to get metadata for binary path '{binary_path}': {inner_error}")]
    BinaryPathMeta {
        binary_path: String,
        inner_error: AnyError,
    },
    #[error(
        "Invalid file type for binary '{binary_path}': expected {expected:?}, found {actual:?}"
    )]
    InvalidFileType {
        binary_path: String,
        actual: FileType,
        expected: FileType,
    },
    #[error("Binary '{binary_path}' is not executable (mode: {mode}, mask: {mask})")]
    NotExecutable {
        binary_path: String,
        mode: String,
        mask: String,
    },
    #[error("Unable to resolve $PATH variable for search paths: {inner_error}")]
    PathUnset { inner_error: AnyError },
}

#[instrument(skip(search_path), fields(search_path = %search_path.display(), search_name = %search_name))]
#[allow(clippy::too_many_lines)]
async fn scan_path(
    search_path: PathBuf,
    search_name: String,
) -> Result<Option<PathBuf>, FindBinaryError> {
    tracing::debug!(
        search_path = %search_path.display(),
        search_name = %search_name,
        "Scanning path for binary"
    );
    let search_path = tokio::fs::canonicalize(&search_path)
        .await
        .map_err(|e| FindBinaryError::SearchPathCanonicalize {
            search_path: search_path.display().to_string(),
            inner_error: e.into(),
        })
        .inspect(|canonicalized_path| {
            tracing::trace!(
                search_path = %search_path.display(),
                canonicalized_path = %canonicalized_path.display(),
                "Canonicalized search path"
            );
        })
        .inspect_err(|e| {
            tracing::warn!(
                original_path = %search_path.display(),
                error = %e,
                "Failed to canonicalize search path"
            );
        })?;

    let metadata = tokio::fs::metadata(&search_path)
        .await
        .map_err(|e| FindBinaryError::SearchPathMetadata {
            search_path: search_path.display().to_string(),
            inner_error: e.into(),
        })
        .inspect(|metadata| {
            tracing::trace!(
                search_path = %search_path.display(),
                metadata = ?metadata,
                "Got metadata for search path"
            );
        })
        .inspect_err(|e| {
            tracing::warn!(
                search_path = %search_path.display(),
                error = %e,
                "Failed to get metadata for search path"
            );
        })?;

    if !metadata.is_dir() {
        // TODO: Want to return an error? I think "not a dir => not found" is pretty clear
        tracing::debug!(
            search_path = %search_path.display(),
            "Search path is not a directory, skipping"
        );
        return Ok(None);
    }

    let mut reader = tokio::fs::read_dir(&search_path)
        .await
        .map_err(|e| FindBinaryError::OpenReadDir {
            search_path: search_path.display().to_string(),
            inner_error: e.into(),
        })
        .inspect_err(|e| {
            tracing::warn!(
                search_path = %search_path.display(),
                error = %e,
                "Failed to open directory for reading"
            );
        })?;

    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| FindBinaryError::ReadDirEntry {
            search_path: search_path.display().to_string(),
            inner_error: e.into(),
        })
        .inspect_err(|e| {
            tracing::warn!(
                search_path = %search_path.display(),
                error = %e,
                "Failed to read directory entry"
            );
        })?
    {
        let entry_name = entry.file_name().to_string_lossy().to_string();
        tracing::trace!(
            entry_name = %entry_name,
            search_name = %search_name,
            "Checking directory entry"
        );

        if entry_name == search_name {
            tracing::debug!(
                binary_path = %entry.path().display(),
                search_name = %search_name,
                "Found matching binary, validating"
            );

            let path = validate_binary(entry.path()).await?;

            tracing::info!(
                binary_path = %path.display(),
                search_name = %search_name,
                "Successfully found and validated binary"
            );

            return Ok(Some(path));
        }
    }

    tracing::debug!(
        search_path = %search_path.display(),
        search_name = %search_name,
        "Binary not found in this path"
    );

    Ok(None)
}

#[instrument(skip(path), fields(binary_path = %path.as_ref().display()))]
async fn validate_binary<P: AsRef<Path>>(path: P) -> Result<PathBuf, FindBinaryError> {
    tracing::debug!(
        binary_path = %path.as_ref().display(),
        "Validating binary"
    );

    let path = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| FindBinaryError::BinaryPathCanonicalize {
            binary_path: path.as_ref().display().to_string(),
            inner_error: e.into(),
        })
        .inspect(|canonicalized_path| {
            tracing::trace!(
                canonicalized_path = %canonicalized_path.display(),
                "Binary path canonicalized successfully"
            );
        })
        .inspect_err(|e| {
            tracing::warn!(
                binary_path = %path.as_ref().display(),
                error = %e,
                "Failed to canonicalize binary path"
            );
        })?;

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| FindBinaryError::BinaryPathMeta {
            binary_path: path.display().to_string(),
            inner_error: e.into(),
        })
        .inspect_err(|e| {
            tracing::warn!(
                binary_path = %path.display(),
                error = %e,
                "Failed to get metadata for binary path"
            );
        })?;

    if !metadata.is_file() {
        let file_type = metadata.file_type().into();

        tracing::warn!(
            binary_path = %path.display(),
            file_type = ?file_type,
            "Binary path is not a regular file"
        );

        return Err(FindBinaryError::InvalidFileType {
            binary_path: path.display().to_string(),
            expected: FileType::File,
            actual: file_type,
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = metadata.mode();
        let mask = 0o111;

        tracing::trace!(
            binary_path = %path.display(),
            mode = format!("{mode:o}"),
            mask = format!("{mask:o}"),
            "Checking executable permissions"
        );

        // TODO: Check that current user has group,user perms, not just that the binary is executable? maybe?

        if mode & mask == 0 {
            tracing::warn!(
                binary_path = %path.display(),
                mode = format!("{mode:o}"),
                mask = format!("{mask:o}"),
                "Binary is not executable"
            );

            return Err(FindBinaryError::NotExecutable {
                binary_path: path.display().to_string(),
                mode: format!("{mode:o}"),
                mask: format!("{mask:o}"),
            });
        }

        tracing::trace!(
            binary_path = %path.display(),
            mode = format!("{mode:o}"),
            "Binary has executable permissions"
        );
    }

    // TODO: Non-unix perms check? idk how windows works lol

    tracing::debug!(
        binary_path = %path.display(),
        "Binary validation successful"
    );

    Ok(path)
}

#[instrument(fields(binary_name = %name, has_given_path = given_path.is_some()))]
pub async fn find_binary(
    name: &str,
    search_paths: String,
    given_path: Option<PathBuf>,
) -> Result<Option<PathBuf>, FindBinaryError> {
    tracing::info!(
        binary_name = %name,
        has_given_path = given_path.is_some(),
        "Starting binary search"
    );

    // Check given path first
    if let Some(given_path) = given_path {
        tracing::debug!(
            binary_name = %name,
            given_path = %given_path.display(),
            "Checking given path"
        );

        match validate_binary(&given_path).await {
            Ok(path) => {
                tracing::info!(
                    binary_name = %name,
                    binary_path = %path.display(),
                    "Found binary at given path"
                );

                return Ok(Some(path));
            }
            Err(e) => {
                tracing::warn!(
                    binary_name = %name,
                    given_path = %given_path.display(),
                    error =% e,
                    "Unable to validate given path"
                );
            }
        }
    }

    // Then scan search_paths
    let search_paths = std::env::split_paths(&search_paths).collect::<Vec<_>>();

    tracing::debug!(
        binary_name = %name,
        path_count = search_paths.len(),
        "Scanning search paths"
    );

    let mut search_tasks = JoinSet::new();
    let current_span = Span::current();
    for path in search_paths {
        let name = name.to_string();
        let span = tracing::debug_span!(parent: &current_span, "scan_task", path =% path.display(), name =% name);
        search_tasks.spawn(scan_path(path, name).instrument(span));
    }

    // TODO: what to do about errors
    while let Some(next) = search_tasks.join_next().await {
        match next {
            Ok(Ok(Some(path))) => {
                tracing::info!(
                    binary_name = %name,
                    binary_path = %path.display(),
                    "Binary found in search paths"
                );
                return Ok(Some(path));
            }
            Ok(Ok(None)) => {
                tracing::trace!("Search task completed with no result");
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Failed to search PATH directory: {e}");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to join search task: {e}");
            }
        }
    }

    tracing::warn!(
        binary_name = %name,
        "Binary not found in any search paths"
    );

    Ok(None)
}

#[instrument(fields(binary_name = %name))]
pub async fn find_binary_env(name: &str) -> Result<Option<PathBuf>, FindBinaryError> {
    let env_key = format!("LIBFFMPEG_{}_PATH", name.to_ascii_uppercase());

    tracing::debug!(
        binary_name = %name,
        env_key = %env_key,
        "Searching for binary using environment variables"
    );

    let env_var = match std::env::var(&env_key) {
        Ok(var) if !var.trim().is_empty() => {
            tracing::info!(
                env_key = %env_key,
                env_value = %var,
                "Found environment variable with explicit path"
            );
            Some(PathBuf::from(var.trim().to_string()))
        }
        Ok(_) => {
            tracing::debug!(
                env_key = %env_key,
                "Environment variable not set"
            );
            None
        }
        Err(e) => {
            tracing::debug!(
                env_key = %env_key,
                error = %e,
                "Environment variable not set"
            );
            None
        }
    };

    let search_paths = std::env::var("PATH").map_err(|e| {
        tracing::error!(
            error = %e,
            "Failed to retrieve $PATH environment variable"
        );
        FindBinaryError::PathUnset {
            inner_error: e.into(),
        }
    })?;

    tracing::trace!(
        path_value = %search_paths,
        "Retrieved $PATH environment variable"
    );

    find_binary(name, search_paths, env_var).await
}
