use std::path::{Component, Path, PathBuf};

use path_clean::PathClean;
use relative_path::RelativePath;
use serde::{Deserialize, Serialize};

pub mod local;
pub mod selection;
mod upload;
mod watcher;

use crate::utils::{
    errors::ServiceError,
    paths::{is_protected_system_path, resolve_existing_ancestor},
};
use local::LocalStorage;
pub(crate) use upload::MAX_UPLOAD_REQUEST_SIZE;
pub use upload::{UploadStatus, UploadStatusQuery};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PathObject {
    pub source: String,
    parent: Option<String>,
    parent_folders: Option<Vec<String>>,
    folders: Option<Vec<String>>,
    files: Option<Vec<VideoFile>>,
    #[serde(default)]
    pub folders_only: bool,
    #[serde(default)]
    pub recursive: bool,
    /// Probe ffprobe durations for the file list. Disable for fast listings
    /// that only need file names (durations default to 0.0).
    #[serde(default = "default_true")]
    pub with_duration: bool,
}

fn default_true() -> bool {
    true
}

impl PathObject {
    fn new(source: String, parent: Option<String>) -> Self {
        Self {
            source,
            parent,
            parent_folders: Some(vec![]),
            folders: Some(vec![]),
            files: Some(vec![]),
            folders_only: false,
            recursive: false,
            with_duration: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MoveObject {
    source: String,
    target: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VideoFile {
    name: String,
    duration: f64,
}

pub async fn init_storage(extensions: Vec<String>) -> Result<LocalStorage, ServiceError> {
    // There is no fixed media root anymore; the browser works with absolute
    // paths from any disk. The current directory is only a fallback anchor
    // for legacy relative sources.
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    LocalStorage::new(root, extensions).await
}

/// Normalize absolute paths for browsing
///
/// Relative inputs are anchored to `root_path` and may not leave it (symlink
/// escapes are detected). Absolute inputs that do not resolve inside the
/// storage root are browsed as-is, so media no longer has to live inside the
/// channel storage. Protected system directories are rejected in both cases.
pub fn norm_abs_path(
    root_path: &Path,
    input_path: &str,
) -> Result<(PathBuf, String, String), ServiceError> {
    if Path::new(input_path)
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ServiceError::Forbidden("Access denied".to_string()));
    }

    let path_relative = RelativePath::new(&root_path.to_string_lossy())
        .normalize()
        .to_string();
    let path_suffix = root_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let mut source_relative = RelativePath::new(input_path).normalize().to_string();

    if Path::new(&source_relative).starts_with(&path_relative) {
        source_relative = source_relative
            .strip_prefix(&path_relative)
            .and_then(|suffix| suffix.strip_prefix(['/', '\\']))
            .unwrap_or_default()
            .to_string();
    } else {
        source_relative = source_relative
            .strip_prefix(&path_suffix)
            .and_then(|s| s.strip_prefix(['/', '\\']))
            .unwrap_or(&source_relative)
            .to_string();
    }

    let anchored = root_path.join(&source_relative).clean();
    let cleaned_root = root_path.clean();

    // An absolute input outside the storage root is browsed where it points,
    // instead of being forced below the storage root.
    let use_absolute = Path::new(input_path).is_absolute()
        && ensure_path_within_root(&cleaned_root, &anchored).is_err();

    if use_absolute {
        let absolute = Path::new(input_path).clean();

        if is_protected_system_path(&absolute) {
            return Err(ServiceError::Forbidden("Access denied".to_string()));
        }

        let suffix = absolute
            .file_name()
            .unwrap_or_else(|| absolute.as_os_str())
            .to_string_lossy()
            .to_string();

        // Forward slashes keep the source usable as a URL fragment and match
        // the path style the frontend builds for navigation.
        let source = absolute.to_string_lossy().replace('\\', "/");

        return Ok((absolute, suffix, source));
    }

    // Defensive containment check: a relative input must never leave the
    // storage root, regardless of the normalization above.
    ensure_path_within_root(&cleaned_root, &anchored)?;

    if is_protected_system_path(&anchored) {
        return Err(ServiceError::Forbidden("Access denied".to_string()));
    }

    Ok((anchored, path_suffix, source_relative))
}

pub(crate) fn ensure_path_within_root(root: &Path, path: &Path) -> Result<(), ServiceError> {
    let root = root.clean();
    let path = path.clean();

    if !path.starts_with(&root) {
        return Err(ServiceError::Forbidden("Access denied".to_string()));
    }

    let resolved_root = resolve_existing_ancestor(&root)?;
    let resolved_path = resolve_existing_ancestor(&path)?;

    if !resolved_path.starts_with(resolved_root) {
        return Err(ServiceError::Forbidden("Access denied".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_paths_preserve_virtual_roots_and_use_component_boundaries() {
        let root = std::env::temp_dir().join(format!("ffplayout-paths-{}", uuid::Uuid::new_v4()));

        for input in ["", "/", "video.mp4", "/video.mp4", "folder/video.mp4"] {
            let (path, _, _) = norm_abs_path(&root, input).unwrap();

            assert!(path.starts_with(&root), "{input}");
        }

        let file = root.join("video.mp4");
        assert_eq!(
            norm_abs_path(&root, &file.to_string_lossy()).unwrap().0,
            file
        );
        let similar_root = format!("{}-other/video.mp4", root.display());
        let (path, _, _) = norm_abs_path(&root, &similar_root).unwrap();

        // Absolute paths outside the storage root are browsed as-is now,
        // instead of being rejected.
        assert_eq!(path, Path::new(&similar_root).clean());
        assert!(!path.starts_with(&root));
    }

    #[test]
    fn normalized_paths_reject_parent_traversal() {
        let root = std::env::temp_dir().join("ffplayout-storage");

        for input in [
            "..",
            "../secret",
            "folder/../../secret",
            "folder/../video.mp4",
            "/../secret",
        ] {
            assert!(
                matches!(norm_abs_path(&root, input), Err(ServiceError::Forbidden(_))),
                "{input}"
            );
        }
    }

    #[test]
    fn drive_roots_and_their_folders_are_browsable() {
        let root = std::env::current_dir().unwrap();
        let drive = if cfg!(windows) { "C:/" } else { "/" };

        assert!(norm_abs_path(&root, drive).is_ok());
        assert!(norm_abs_path(&root, &format!("{drive}Users")).is_ok());
        assert!(norm_abs_path(&root, &format!("{drive}Users/Leonardi/Videos")).is_ok());
    }

    #[test]
    fn absolute_paths_are_browsed_as_is_outside_the_storage_root() {
        let root =
            std::env::temp_dir().join(format!("ffplayout-absolute-{}", uuid::Uuid::new_v4()));
        let storage = root.join("storage");
        let external = root.join("external");
        std::fs::create_dir_all(storage.join("inside")).unwrap();
        std::fs::create_dir_all(external.join("show")).unwrap();

        let inside = external.join("show").join("clip.mp4");
        let resolved = norm_abs_path(&storage, &inside.to_string_lossy())
            .unwrap()
            .0;

        assert_eq!(resolved, inside);

        let protected = if cfg!(windows) { r"C:\Windows" } else { "/etc" };

        assert!(matches!(
            norm_abs_path(&storage, protected),
            Err(ServiceError::Forbidden(_))
        ));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn containment_checks_symlinks_for_existing_files_and_new_targets() {
        let directory =
            std::env::temp_dir().join(format!("ffplayout-symlinks-{}", uuid::Uuid::new_v4()));
        let root = directory.join("storage");
        let outside = directory.join("outside");
        std::fs::create_dir_all(root.join("inside")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        std::os::unix::fs::symlink(root.join("inside"), root.join("safe")).unwrap();
        std::os::unix::fs::symlink(outside.join("missing"), root.join("dangling")).unwrap();

        assert!(norm_abs_path(&root, "escape/secret").is_err());
        assert!(norm_abs_path(&root, "escape/new-file").is_err());
        assert!(norm_abs_path(&root, "dangling").is_err());
        assert!(norm_abs_path(&root, "safe/new-file").is_ok());
        assert!(ensure_path_within_root(&root, &outside.join("secret")).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
