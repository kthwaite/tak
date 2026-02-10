use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathNormalizationError {
    EmptyInput,
    EscapesRepositoryRoot,
    AbsoluteOutsideRepository,
    ResolvesToRepositoryRoot,
    InvalidRepositoryRoot,
}

impl std::fmt::Display for PathNormalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "path cannot be empty"),
            Self::EscapesRepositoryRoot => {
                write!(f, "path traversal escapes above repository root")
            }
            Self::AbsoluteOutsideRepository => {
                write!(f, "absolute path is outside the repository root")
            }
            Self::ResolvesToRepositoryRoot => {
                write!(f, "path resolves to repository root and is not allowed")
            }
            Self::InvalidRepositoryRoot => write!(f, "repository root path is invalid"),
        }
    }
}

impl std::error::Error for PathNormalizationError {}

/// Normalize a reservation path to a canonical repo-relative representation.
///
/// Contract (RFC 0001 addendum):
/// - trim surrounding whitespace
/// - normalize separators to '/'
/// - collapse duplicate separators
/// - resolve '.' and '..' lexically
/// - reject traversal that escapes above repo root
/// - convert absolute-in-repo paths to repo-relative
/// - reject absolute paths outside repo root
/// - remove leading './' and trailing '/'
pub fn normalize_reservation_path(
    input: &str,
    repo_root: &Path,
) -> Result<String, PathNormalizationError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(PathNormalizationError::EmptyInput);
    }

    let normalized_input = normalize_separators(input);
    let parsed = ParsedPath::parse(&normalized_input);

    let normalized_segments = if parsed.absolute {
        normalize_absolute(parsed, repo_root)?
    } else {
        lexical_normalize(parsed.segments, true)?
    };

    if normalized_segments.is_empty() {
        return Err(PathNormalizationError::ResolvesToRepositoryRoot);
    }

    Ok(normalized_segments.join("/"))
}

/// Comparison key for canonical reservation paths.
///
/// Windows compares case-insensitively, Unix/macOS compares case-sensitively.
pub fn path_conflict_key(path: &str) -> String {
    let collapsed = normalize_separators(path)
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");

    if cfg!(windows) {
        collapsed.to_ascii_lowercase()
    } else {
        collapsed
    }
}

/// Conflict predicate for already-normalized repo-relative paths.
///
/// Two paths conflict iff:
/// 1) keys are equal, or
/// 2) one key is a segment-bounded prefix of the other.
pub fn normalized_paths_conflict(a: &str, b: &str) -> bool {
    let a_key = path_conflict_key(a);
    let b_key = path_conflict_key(b);

    if a_key.is_empty() || b_key.is_empty() {
        return false;
    }

    if a_key == b_key {
        return true;
    }

    b_key.starts_with(&format!("{a_key}/")) || a_key.starts_with(&format!("{b_key}/"))
}

#[derive(Debug, Clone)]
struct ParsedPath {
    drive: Option<String>,
    absolute: bool,
    segments: Vec<String>,
}

impl ParsedPath {
    fn parse(normalized: &str) -> Self {
        let mut rest = normalized;
        let mut drive = None;

        if has_windows_drive_prefix(rest) {
            drive = Some(rest[..2].to_string());
            rest = &rest[2..];
        }

        let absolute = rest.starts_with('/');
        if absolute {
            rest = rest.trim_start_matches('/');
        }

        let segments = if rest.is_empty() {
            vec![]
        } else {
            rest.split('/').map(|segment| segment.to_string()).collect()
        };

        Self {
            drive,
            absolute,
            segments,
        }
    }
}

fn normalize_absolute(
    path: ParsedPath,
    repo_root: &Path,
) -> Result<Vec<String>, PathNormalizationError> {
    let repo_root = absolute_repo_root(repo_root)?;
    let repo_root_norm = normalize_separators(&repo_root.to_string_lossy());
    let repo_parsed = ParsedPath::parse(&repo_root_norm);

    if !repo_parsed.absolute {
        return Err(PathNormalizationError::InvalidRepositoryRoot);
    }

    if !drive_matches(path.drive.as_deref(), repo_parsed.drive.as_deref()) {
        return Err(PathNormalizationError::AbsoluteOutsideRepository);
    }

    let path_segments = lexical_normalize(path.segments, false)?;
    let root_segments = lexical_normalize(repo_parsed.segments, false)?;

    if path_segments.len() < root_segments.len() {
        return Err(PathNormalizationError::AbsoluteOutsideRepository);
    }

    if !segments_prefix_match(&path_segments, &root_segments) {
        return Err(PathNormalizationError::AbsoluteOutsideRepository);
    }

    Ok(path_segments[root_segments.len()..].to_vec())
}

fn absolute_repo_root(repo_root: &Path) -> Result<PathBuf, PathNormalizationError> {
    if repo_root.is_absolute() {
        return Ok(repo_root.to_path_buf());
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(repo_root))
        .map_err(|_| PathNormalizationError::InvalidRepositoryRoot)
}

fn lexical_normalize(
    segments: Vec<String>,
    reject_escape: bool,
) -> Result<Vec<String>, PathNormalizationError> {
    let mut normalized = Vec::new();

    for segment in segments {
        match segment.as_str() {
            "" | "." => {}
            ".." => {
                if normalized.pop().is_none() && reject_escape {
                    return Err(PathNormalizationError::EscapesRepositoryRoot);
                }
            }
            _ => normalized.push(segment),
        }
    }

    Ok(normalized)
}

fn normalize_separators(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_sep = false;

    for ch in input.chars() {
        let is_sep = ch == '/' || ch == '\\';
        if is_sep {
            if !prev_sep {
                out.push('/');
            }
            prev_sep = true;
        } else {
            out.push(ch);
            prev_sep = false;
        }
    }

    out
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn drive_matches(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(lhs), Some(rhs)) => {
            if cfg!(windows) {
                lhs.eq_ignore_ascii_case(rhs)
            } else {
                lhs == rhs
            }
        }
        (None, None) => true,
        _ => false,
    }
}

fn segments_prefix_match(path: &[String], root: &[String]) -> bool {
    root.iter()
        .zip(path.iter())
        .all(|(lhs, rhs)| segment_eq(lhs, rhs))
}

fn segment_eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn normalize_relative_paths() {
        let dir = tempdir().unwrap();

        assert_eq!(
            normalize_reservation_path("src/./store/mesh.rs", dir.path()).unwrap(),
            "src/store/mesh.rs"
        );
        assert_eq!(
            normalize_reservation_path("./src/store/", dir.path()).unwrap(),
            "src/store"
        );
        assert_eq!(
            normalize_reservation_path("src\\store\\mesh.rs", dir.path()).unwrap(),
            "src/store/mesh.rs"
        );
        assert_eq!(
            normalize_reservation_path("src//store///mesh.rs", dir.path()).unwrap(),
            "src/store/mesh.rs"
        );
    }

    #[test]
    fn normalize_rejects_invalid_relative_inputs() {
        let dir = tempdir().unwrap();

        assert_eq!(
            normalize_reservation_path("   ", dir.path()).unwrap_err(),
            PathNormalizationError::EmptyInput
        );
        assert_eq!(
            normalize_reservation_path("../src/store", dir.path()).unwrap_err(),
            PathNormalizationError::EscapesRepositoryRoot
        );
        assert_eq!(
            normalize_reservation_path("src/../../store", dir.path()).unwrap_err(),
            PathNormalizationError::EscapesRepositoryRoot
        );
        assert_eq!(
            normalize_reservation_path(".", dir.path()).unwrap_err(),
            PathNormalizationError::ResolvesToRepositoryRoot
        );
    }

    #[test]
    fn normalize_absolute_in_repo_to_relative() {
        let dir = tempdir().unwrap();
        let input = dir.path().join("src").join("mesh").join("mod.rs");

        let normalized = normalize_reservation_path(&input.to_string_lossy(), dir.path()).unwrap();
        assert_eq!(normalized, "src/mesh/mod.rs");
    }

    #[test]
    fn normalize_rejects_absolute_outside_repo() {
        let repo = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let input = outside.path().join("src").join("mesh.rs");

        let err = normalize_reservation_path(&input.to_string_lossy(), repo.path()).unwrap_err();
        assert_eq!(err, PathNormalizationError::AbsoluteOutsideRepository);
    }

    #[test]
    fn normalized_paths_conflict_segment_bounded() {
        assert!(normalized_paths_conflict("src/store", "src/store/mesh.rs"));
        assert!(normalized_paths_conflict("src/store/mesh.rs", "src/store"));
        assert!(normalized_paths_conflict("src/store", "src/store"));
        assert!(!normalized_paths_conflict("src/store", "src/storehouse"));
        assert!(!normalized_paths_conflict("src/a.rs", "src/b.rs"));
    }

    #[test]
    fn normalized_paths_conflict_honors_platform_case_rules() {
        if cfg!(windows) {
            assert!(normalized_paths_conflict("SRC/Store", "src/store/mesh.rs"));
        } else {
            assert!(!normalized_paths_conflict("SRC/Store", "src/store/mesh.rs"));
        }
    }

    #[test]
    fn reservation_matching_normalizes_dot_segments_before_conflict_checks() {
        let repo = tempdir().unwrap();

        let held = normalize_reservation_path("src/store/./mesh/..", repo.path()).unwrap();
        let requested =
            normalize_reservation_path("./src/store/../store/mesh.rs", repo.path()).unwrap();
        let unrelated = normalize_reservation_path("src/storehouse/file.rs", repo.path()).unwrap();

        assert_eq!(held, "src/store");
        assert_eq!(requested, "src/store/mesh.rs");
        assert!(normalized_paths_conflict(&held, &requested));
        assert!(!normalized_paths_conflict(&held, &unrelated));
    }

    #[test]
    fn reservation_matching_case_behavior_after_normalization() {
        let repo = tempdir().unwrap();

        let held = normalize_reservation_path("SRC/Store", repo.path()).unwrap();
        let requested = normalize_reservation_path("src/store/mesh.rs", repo.path()).unwrap();

        if cfg!(windows) {
            assert!(normalized_paths_conflict(&held, &requested));
        } else {
            assert!(!normalized_paths_conflict(&held, &requested));
        }
    }

    #[cfg(unix)]
    #[test]
    fn reservation_matching_is_lexical_for_symlink_paths() {
        let repo = tempdir().unwrap();
        let real_dir = repo.path().join("real");
        let real_file = real_dir.join("mesh.rs");
        let link_dir = repo.path().join("link");
        let link_file = link_dir.join("mesh.rs");

        fs::create_dir_all(&real_dir).unwrap();
        fs::write(&real_file, "// real\n").unwrap();
        symlink(&real_dir, &link_dir).unwrap();

        let normalized_link =
            normalize_reservation_path(&link_file.to_string_lossy(), repo.path()).unwrap();
        let normalized_real =
            normalize_reservation_path(&real_file.to_string_lossy(), repo.path()).unwrap();

        assert_eq!(normalized_link, "link/mesh.rs");
        assert_eq!(normalized_real, "real/mesh.rs");
        assert!(!normalized_paths_conflict(
            &normalized_link,
            &normalized_real
        ));
    }
}
