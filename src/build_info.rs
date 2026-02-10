/// Build-time git commit SHA stamped by build.rs when available.
pub fn git_sha() -> Option<&'static str> {
    option_env!("TAK_BUILD_GIT_SHA")
}
