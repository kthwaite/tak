use std::path::Path;

use git2::Repository;

/// Information about the current HEAD: branch name and commit SHA.
pub struct HeadInfo {
    pub branch: Option<String>,
    pub sha: String,
}

/// Retrieve the current HEAD's branch name and commit SHA.
/// Returns None if the repo root is not inside a git repository.
pub fn current_head_info(repo_root: &Path) -> Option<HeadInfo> {
    let repo = Repository::discover(repo_root).ok()?;
    let head = repo.head().ok()?;
    let branch = if head.is_branch() {
        head.shorthand().map(String::from)
    } else {
        None
    };
    let sha = head.peel_to_commit().ok()?.id().to_string();
    Some(HeadInfo { branch, sha })
}

/// Return one-line commit summaries between `start_sha` (exclusive) and
/// `end_sha` (inclusive). Falls back to an empty vec on any error (detached
/// HEAD, shallow clone, non-git repo, etc.).
pub fn commits_since(repo_root: &Path, start_sha: &str, end_sha: &str) -> Vec<String> {
    let Ok(repo) = Repository::discover(repo_root) else {
        return vec![];
    };
    let Ok(start_oid) = git2::Oid::from_str(start_sha) else {
        return vec![];
    };
    let Ok(end_oid) = git2::Oid::from_str(end_sha) else {
        return vec![];
    };
    let Ok(mut revwalk) = repo.revwalk() else {
        return vec![];
    };
    if revwalk.push(end_oid).is_err() {
        return vec![];
    }
    if revwalk.hide(start_oid).is_err() {
        return vec![];
    }

    let mut summaries = Vec::new();
    for oid in revwalk {
        let Ok(oid) = oid else { continue };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let summary = commit.summary().unwrap_or("(no message)").to_string();
        let short = &commit.id().to_string()[..7];
        summaries.push(format!("{short} {summary}"));
    }
    summaries
}
