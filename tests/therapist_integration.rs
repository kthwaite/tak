#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tempfile::tempdir;

use tak::output::Format;
use tak::store::files::FileStore;
use tak::store::therapist::{TherapistMode, TherapistStore};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn set_value(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.as_deref() {
            Some(value) => unsafe {
                std::env::set_var(self.key, value);
            },
            None => unsafe {
                std::env::remove_var(self.key);
            },
        }
    }
}

fn write_mock_pi_script(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let script_path = dir.join("mock-pi.sh");
    fs::write(&script_path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();

    let mut perms = fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).unwrap();

    script_path
}

#[test]
fn therapist_online_with_mocked_rpc_process_records_observation() {
    let _lock = ENV_LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let session_path = dir.path().join("work-loop-session.jsonl");
    fs::write(
        &session_path,
        "{\"type\":\"user\",\"text\":\"/tak work start\"}\n",
    )
    .unwrap();

    let mock_pi = write_mock_pi_script(
        dir.path(),
        r#"while IFS= read -r line; do
  case "$line" in
    *\"id\":\"therapist-prompt\"*)
      echo '{"type":"response","id":"therapist-prompt","command":"prompt","success":true}'
      echo '{"type":"agent_end","messages":[]}'
      ;;
    *\"id\":\"therapist-last\"*)
      echo '{"type":"response","id":"therapist-last","command":"get_last_assistant_text","success":true,"data":{"text":"- We should reduce reservation churn\\n- Recommend a clearer handoff template"}}'
      ;;
  esac
done"#,
    );

    let _env = EnvVarGuard::set_path("TAK_THERAPIST_PI_BIN", &mock_pi);

    tak::commands::therapist::online(
        dir.path(),
        Some(session_path.display().to_string()),
        None,
        Some("agent-test".into()),
        Format::Json,
    )
    .unwrap();

    let store = TherapistStore::open(&dir.path().join(".tak"));
    let observations = store.list(None).unwrap();
    assert_eq!(observations.len(), 1);

    let obs = &observations[0];
    assert_eq!(obs.mode, TherapistMode::Online);
    assert_eq!(obs.requested_by.as_deref(), Some("agent-test"));
    assert!(
        obs.session
            .as_deref()
            .unwrap_or_default()
            .ends_with("work-loop-session.jsonl")
    );
    assert!(
        obs.interview
            .as_deref()
            .unwrap_or_default()
            .contains("reservation churn")
    );
    assert!(!obs.recommendations.is_empty());
}

#[test]
fn therapist_online_propagates_rpc_prompt_failure() {
    let _lock = ENV_LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let session_path = dir.path().join("failed-session.jsonl");
    fs::write(
        &session_path,
        "{\"type\":\"user\",\"text\":\"/tak work\"}\n",
    )
    .unwrap();

    let mock_pi = write_mock_pi_script(
        dir.path(),
        r#"while IFS= read -r line; do
  case "$line" in
    *\"id\":\"therapist-prompt\"*)
      echo '{"type":"response","id":"therapist-prompt","command":"prompt","success":false,"error":"mock prompt failure"}'
      exit 0
      ;;
  esac
done"#,
    );

    let _env = EnvVarGuard::set_path("TAK_THERAPIST_PI_BIN", &mock_pi);

    let err = tak::commands::therapist::online(
        dir.path(),
        Some(session_path.display().to_string()),
        None,
        Some("agent-test".into()),
        Format::Json,
    )
    .unwrap_err();

    assert!(err.to_string().contains("mock prompt failure"));

    let store = TherapistStore::open(&dir.path().join(".tak"));
    assert!(store.list(None).unwrap().is_empty());
}

#[test]
fn therapist_online_times_out_when_rpc_never_replies() {
    let _lock = ENV_LOCK.lock().unwrap();

    let dir = tempdir().unwrap();
    FileStore::init(dir.path()).unwrap();

    let session_path = dir.path().join("timeout-session.jsonl");
    fs::write(
        &session_path,
        "{\"type\":\"user\",\"text\":\"/tak work\"}\n",
    )
    .unwrap();

    let mock_pi = write_mock_pi_script(
        dir.path(),
        r#"while IFS= read -r _line; do
  sleep 2
done"#,
    );

    let _pi = EnvVarGuard::set_path("TAK_THERAPIST_PI_BIN", &mock_pi);
    let _phase_timeout = EnvVarGuard::set_value("TAK_THERAPIST_RPC_PHASE_TIMEOUT_MS", "120");
    let _total_timeout = EnvVarGuard::set_value("TAK_THERAPIST_RPC_TOTAL_TIMEOUT_MS", "240");

    let started = Instant::now();
    let err = tak::commands::therapist::online(
        dir.path(),
        Some(session_path.display().to_string()),
        None,
        Some("agent-timeout".into()),
        Format::Json,
    )
    .unwrap_err();

    assert!(err.to_string().contains("timed out") || err.to_string().contains("timeout"));
    assert!(started.elapsed() < Duration::from_secs(2));

    let store = TherapistStore::open(&dir.path().join(".tak"));
    assert!(store.list(None).unwrap().is_empty());
}
