/// Resolve the agent identity from the environment.
///
/// Checks `TAK_AGENT` env var first. Returns `None` if unset,
/// letting callers decide whether to fall back or leave assignee empty.
pub fn resolve_agent() -> Option<String> {
    std::env::var("TAK_AGENT").ok().filter(|s| !s.is_empty())
}

const ADJECTIVES: &[&str] = &[
    "brisk", "calm", "clever", "daring", "eager", "fierce", "gentle", "jolly", "keen", "lively",
    "mellow", "nimble", "plucky", "quick", "spry", "vivid",
];

const ANIMALS: &[&str] = &[
    "otter", "lynx", "falcon", "badger", "fox", "panda", "koala", "tiger", "yak", "heron", "orca",
    "beaver", "raven", "gecko", "walrus", "cougar",
];

fn codename_from_uuid(id: uuid::Uuid) -> String {
    let bytes = id.as_bytes();
    let adjective = ADJECTIVES[(bytes[0] as usize) % ADJECTIVES.len()];
    let animal = ANIMALS[(bytes[1] as usize) % ANIMALS.len()];
    let suffix = format!("{:02x}{:02x}", bytes[2], bytes[3]);
    format!("{adjective}-{animal}-{suffix}")
}

/// Auto-generated fallback for contexts that require an assignee (e.g. `claim`).
///
/// Uses a memorable adjective-animal codename with a short hex suffix,
/// for example: `nimble-otter-7fa2`.
pub fn generated_fallback() -> String {
    codename_from_uuid(uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var tests must not run concurrently.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn generated_fallback_uses_adjective_animal_style() {
        let f = generated_fallback();
        let parts: Vec<&str> = f.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(ADJECTIVES.contains(&parts[0]));
        assert!(ANIMALS.contains(&parts[1]));
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn codename_is_deterministic_for_uuid() {
        let id = uuid::Uuid::parse_str("00010203-0000-0000-0000-000000000000").unwrap();
        assert_eq!(codename_from_uuid(id), "brisk-lynx-0203");
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
