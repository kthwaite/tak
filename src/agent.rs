/// Resolve the agent identity from the environment.
///
/// Checks `TAK_AGENT` env var first. Returns `None` if unset,
/// letting callers decide whether to fall back or leave assignee empty.
pub fn resolve_agent() -> Option<String> {
    std::env::var("TAK_AGENT").ok().filter(|s| !s.is_empty())
}

/// PID-based fallback for contexts that require an assignee (e.g. `claim`).
pub fn pid_fallback() -> String {
    format!("pid-{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var tests must not run concurrently.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pid_fallback_is_nonempty() {
        let f = pid_fallback();
        assert!(f.starts_with("pid-"));
        assert!(f.len() > 4);
    }

    #[test]
    fn resolve_agent_env_behavior() {
        let _guard = ENV_LOCK.lock().unwrap();

        // Reads TAK_AGENT when set
        unsafe { std::env::set_var("TAK_AGENT", "test-agent-42") };
        assert_eq!(resolve_agent(), Some("test-agent-42".to_string()));

        // Ignores empty value
        unsafe { std::env::set_var("TAK_AGENT", "") };
        assert_eq!(resolve_agent(), None);

        // None when unset
        unsafe { std::env::remove_var("TAK_AGENT") };
        assert_eq!(resolve_agent(), None);
    }
}
