#!/usr/bin/env python3

import argparse
import dataclasses
import datetime as dt
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


@dataclasses.dataclass
class ExecResult:
    command: str
    exit_code: int
    stdout: str
    stderr: str
    timed_out: bool = False


@dataclasses.dataclass
class ObjectiveConfig:
    objective_id: str
    name: str
    description: str
    root: Path
    template_dir: Path
    initial_prompt_path: Path
    change_prompt_path: Path
    time_budget_minutes: int
    poll_interval_seconds: int
    public_probe_interval_seconds: int
    worker_ready_strategy: str
    worker_ready_delay_seconds: int
    worker_ready_timeout_seconds: int
    worker_ready_token: str
    worker_prompt_transport: str
    hidden_tests_required: bool
    public_test_command: str
    hidden_test_command: str
    change_probe_command: str
    change_min_minutes: int
    change_target_minutes: int
    phase1_done_token: str
    final_done_token: str
    scoring: dict[str, int]


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def sanitize_session_name(name: str) -> str:
    return re.sub(r"[^A-Za-z0-9_]", "_", name)


def append_jsonl(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(payload, ensure_ascii=False) + "\n")


def read_text_safe(path: Path) -> str:
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def run_cmd(
    command: list[str] | str,
    *,
    cwd: Path | None = None,
    timeout: int | None = None,
    shell: bool = False,
    input_text: str | None = None,
    env: dict[str, str] | None = None,
) -> ExecResult:
    command_display = command if isinstance(command, str) else shlex.join(command)
    try:
        completed = subprocess.run(
            command,
            cwd=str(cwd) if cwd else None,
            text=True,
            input=input_text,
            capture_output=True,
            timeout=timeout,
            shell=shell,
            executable="/bin/bash" if shell else None,
            env=env,
            check=False,
        )
        return ExecResult(
            command=command_display,
            exit_code=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
            timed_out=False,
        )
    except subprocess.TimeoutExpired as exc:
        return ExecResult(
            command=command_display,
            exit_code=124,
            stdout=exc.stdout or "",
            stderr=exc.stderr or f"Timed out after {timeout}s",
            timed_out=True,
        )


def ensure_success(result: ExecResult, context: str) -> None:
    if result.exit_code != 0:
        raise RuntimeError(
            f"{context} failed (exit {result.exit_code})\n"
            f"Command: {result.command}\n"
            f"STDOUT:\n{result.stdout}\n"
            f"STDERR:\n{result.stderr}"
        )


def require_binary(name: str) -> None:
    path = shutil.which(name)
    if path is None:
        raise RuntimeError(f"Required binary not found on PATH: {name}")


def running_in_uv_runtime(bench_dir: Path) -> bool:
    if os.environ.get("UV") or os.environ.get("UV_RUN_RECURSION_DEPTH"):
        return True

    try:
        exe = Path(sys.executable).resolve()
        venv_dir = (bench_dir / ".venv").resolve()
        exe.relative_to(venv_dir)
        return True
    except (ValueError, FileNotFoundError):
        return False


def ensure_python_module(module_name: str, install_hint: str) -> None:
    result = run_cmd([sys.executable, "-c", f"import {module_name}"])
    if result.exit_code != 0:
        raise RuntimeError(
            f"Required Python module is not importable: {module_name}\n"
            f"Interpreter: {sys.executable}\n"
            f"Hint: {install_hint}\n"
            f"STDERR: {result.stderr.strip()}"
        )


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []

    events: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        try:
            event = json.loads(stripped)
            if isinstance(event, dict):
                events.append(event)
        except json.JSONDecodeError:
            continue
    return events


def test_command_executed(result: ExecResult | None) -> bool:
    if result is None:
        return False
    if result.timed_out:
        return False
    if result.exit_code == 127:
        return False
    return bool(result.command.strip())


def load_objective(objective_path: Path) -> ObjectiveConfig:
    manifest_path = objective_path
    if objective_path.is_dir():
        manifest_path = objective_path / "objective.toml"

    if not manifest_path.exists():
        raise RuntimeError(f"Objective manifest not found: {manifest_path}")

    raw = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    root = manifest_path.parent.resolve()

    def req(path: str, data: dict[str, Any], expected_type: type) -> Any:
        if path not in data:
            raise RuntimeError(f"Missing required key in {manifest_path}: {path}")
        value = data[path]
        if not isinstance(value, expected_type):
            raise RuntimeError(
                f"Invalid type for {path}: expected {expected_type}, got {type(value)}"
            )
        return value

    paths = raw.get("paths", {})
    worker = raw.get("worker", {})
    change = raw.get("change", {})
    scoring = raw.get("scoring", {})

    worker_ready_strategy = str(worker.get("ready_strategy", "delay")).strip().lower()
    if worker_ready_strategy not in {"delay", "token", "none"}:
        raise RuntimeError(
            "Invalid worker.ready_strategy in objective manifest. "
            "Expected one of: delay, token, none"
        )

    worker_ready_delay_seconds = int(
        worker.get("ready_delay_seconds", raw.get("worker_ready_delay_seconds", 4))
    )
    worker_ready_timeout_seconds = int(worker.get("ready_timeout_seconds", 120))
    worker_ready_token = str(worker.get("ready_token", "")).strip()
    worker_prompt_transport = (
        str(worker.get("prompt_transport", "tmux_buffer_paste")).strip()
        or "tmux_buffer_paste"
    )

    phase1_done_token = str(
        worker.get(
            "phase1_done_token",
            change.get("phase1_done_token", "TAKBENCH_PHASE1_DONE"),
        )
    )
    final_done_token = str(
        worker.get(
            "final_done_token",
            change.get("final_done_token", "TAKBENCH_FINAL_DONE"),
        )
    )

    config = ObjectiveConfig(
        objective_id=req("id", raw, str),
        name=req("name", raw, str),
        description=raw.get("description", ""),
        root=root,
        template_dir=(root / req("template_dir", paths, str)).resolve(),
        initial_prompt_path=(root / req("initial_prompt", paths, str)).resolve(),
        change_prompt_path=(root / req("change_prompt", paths, str)).resolve(),
        time_budget_minutes=int(raw.get("time_budget_minutes", 60)),
        poll_interval_seconds=int(raw.get("poll_interval_seconds", 5)),
        public_probe_interval_seconds=int(raw.get("public_probe_interval_seconds", 90)),
        worker_ready_strategy=worker_ready_strategy,
        worker_ready_delay_seconds=worker_ready_delay_seconds,
        worker_ready_timeout_seconds=worker_ready_timeout_seconds,
        worker_ready_token=worker_ready_token,
        worker_prompt_transport=worker_prompt_transport,
        hidden_tests_required=bool(raw.get("hidden_tests_required", True)),
        public_test_command=req("public_test_command", raw, str),
        hidden_test_command=str(raw.get("hidden_test_command", "")).strip(),
        change_probe_command=str(raw.get("change_probe_command", "")).strip(),
        change_min_minutes=int(change.get("min_minutes", 10)),
        change_target_minutes=int(change.get("target_minutes", 25)),
        phase1_done_token=phase1_done_token,
        final_done_token=final_done_token,
        scoring={k: int(v) for k, v in scoring.items()},
    )

    for path in [config.template_dir, config.initial_prompt_path, config.change_prompt_path]:
        if not path.exists():
            raise RuntimeError(f"Objective path does not exist: {path}")

    if config.worker_ready_delay_seconds < 0:
        raise RuntimeError("worker.ready_delay_seconds must be >= 0")

    if config.worker_ready_timeout_seconds < 1:
        raise RuntimeError("worker.ready_timeout_seconds must be >= 1")

    if config.worker_ready_strategy == "token" and not config.worker_ready_token:
        raise RuntimeError(
            "worker.ready_strategy=token requires a non-empty worker.ready_token"
        )

    return config


def setup_run_dirs(runs_dir: Path, run_id: str) -> dict[str, Path]:
    run_dir = (runs_dir / run_id).resolve()
    repo_dir = run_dir / "repo"
    logs_dir = run_dir / "logs"
    prompts_dir = run_dir / "prompts"

    if run_dir.exists():
        raise RuntimeError(f"Run directory already exists: {run_dir}")

    logs_dir.mkdir(parents=True, exist_ok=False)
    prompts_dir.mkdir(parents=True, exist_ok=False)
    return {
        "run_dir": run_dir,
        "repo_dir": repo_dir,
        "logs_dir": logs_dir,
        "prompts_dir": prompts_dir,
    }


def prepare_repo(
    *,
    repo_dir: Path,
    template_dir: Path,
    init_tak: bool,
) -> str:
    shutil.copytree(template_dir, repo_dir)

    ensure_success(run_cmd(["git", "init", "-b", "main"], cwd=repo_dir), "git init")
    ensure_success(
        run_cmd(["git", "config", "user.name", "takbench"], cwd=repo_dir),
        "git config user.name",
    )
    ensure_success(
        run_cmd(["git", "config", "user.email", "takbench@example.invalid"], cwd=repo_dir),
        "git config user.email",
    )

    if init_tak:
        ensure_success(run_cmd(["tak", "init"], cwd=repo_dir), "tak init")

    ensure_success(run_cmd(["git", "add", "."], cwd=repo_dir), "git add")
    ensure_success(
        run_cmd(["git", "commit", "-m", "Initial benchmark scaffold"], cwd=repo_dir),
        "git commit initial scaffold",
    )

    baseline = run_cmd(["git", "rev-parse", "HEAD"], cwd=repo_dir)
    ensure_success(baseline, "git rev-parse HEAD")
    return baseline.stdout.strip()


def tmux_cmd(args: list[str], *, input_text: str | None = None) -> ExecResult:
    return run_cmd(["tmux", *args], input_text=input_text)


def tmux_send_line(
    pane_target: str,
    line: str,
    commands_log: Path,
    event_type: str = "command",
) -> None:
    ensure_success(
        tmux_cmd(["send-keys", "-t", pane_target, "-l", line]),
        f"tmux send line ({event_type})",
    )
    ensure_success(
        tmux_cmd(["send-keys", "-t", pane_target, "Enter"]),
        f"tmux send enter ({event_type})",
    )

    append_jsonl(
        commands_log,
        {
            "timestamp": utc_now(),
            "type": event_type,
            "target": pane_target,
            "line": line,
        },
    )


def tmux_send_prompt(
    pane_target: str,
    prompt_text: str,
    commands_log: Path,
    event_type: str,
) -> None:
    ensure_success(
        tmux_cmd(["load-buffer", "-"], input_text=prompt_text),
        f"tmux load-buffer ({event_type})",
    )
    ensure_success(
        tmux_cmd(["paste-buffer", "-t", pane_target, "-d"]),
        f"tmux paste-buffer ({event_type})",
    )
    ensure_success(
        tmux_cmd(["send-keys", "-t", pane_target, "Enter"]),
        f"tmux send enter ({event_type})",
    )

    append_jsonl(
        commands_log,
        {
            "timestamp": utc_now(),
            "type": event_type,
            "target": pane_target,
            "prompt": prompt_text,
        },
    )


def start_tmux_session(session: str, pane_log_path: Path) -> str:
    ensure_success(
        tmux_cmd(["new-session", "-d", "-s", session, "-n", "worker"]),
        "tmux new-session",
    )

    pane_lookup = tmux_cmd(
        ["display-message", "-p", "-t", f"{session}:worker", "#{pane_id}"]
    )
    ensure_success(pane_lookup, "tmux pane lookup")
    pane_target = pane_lookup.stdout.strip()

    ensure_success(
        tmux_cmd(["set-option", "-t", session, "history-limit", "200000"]),
        "tmux set history-limit",
    )

    pipe_cmd = f"cat >> {shlex.quote(str(pane_log_path))}"
    ensure_success(
        tmux_cmd(["pipe-pane", "-o", "-t", pane_target, pipe_cmd]),
        "tmux pipe-pane",
    )

    return pane_target


def pane_dead(pane_target: str) -> bool:
    result = tmux_cmd(["display-message", "-p", "-t", pane_target, "#{pane_dead}"])
    if result.exit_code != 0:
        return True
    return result.stdout.strip() == "1"


def capture_pane(pane_target: str, output_path: Path) -> None:
    result = tmux_cmd(["capture-pane", "-t", pane_target, "-S", "-200000", "-p"])
    if result.exit_code == 0:
        output_path.write_text(result.stdout, encoding="utf-8")
    else:
        output_path.write_text(
            f"Failed to capture pane: {result.stderr}",
            encoding="utf-8",
        )


def kill_session(session: str) -> None:
    tmux_cmd(["kill-session", "-t", session])


def wait_for_worker_ready(
    objective: ObjectiveConfig,
    worker_log_path: Path,
) -> dict[str, Any]:
    started = time.time()
    strategy = objective.worker_ready_strategy

    if strategy == "none":
        return {
            "ready": True,
            "strategy": strategy,
            "wait_seconds": 0,
            "reason": "no_wait",
        }

    if strategy == "delay":
        delay_seconds = max(0, objective.worker_ready_delay_seconds)
        if delay_seconds > 0:
            time.sleep(delay_seconds)
        return {
            "ready": True,
            "strategy": strategy,
            "wait_seconds": delay_seconds,
            "reason": "delay_elapsed",
        }

    # strategy == "token"
    token = objective.worker_ready_token
    deadline = started + max(1, objective.worker_ready_timeout_seconds)
    while time.time() < deadline:
        if token in read_text_safe(worker_log_path):
            return {
                "ready": True,
                "strategy": strategy,
                "wait_seconds": time.time() - started,
                "reason": "ready_token_seen",
            }
        time.sleep(1)

    return {
        "ready": False,
        "strategy": strategy,
        "wait_seconds": time.time() - started,
        "reason": "ready_token_timeout",
    }


def exec_shell(command: str, cwd: Path, timeout: int = 300) -> ExecResult:
    return run_cmd(command, cwd=cwd, shell=True, timeout=timeout)


def save_exec_log(path: Path, title: str, result: ExecResult) -> None:
    path.write_text(
        "\n".join(
            [
                f"# {title}",
                f"$ {result.command}",
                f"exit_code: {result.exit_code}",
                "",
                "## stdout",
                result.stdout,
                "",
                "## stderr",
                result.stderr,
                "",
            ]
        ),
        encoding="utf-8",
    )


def parse_iso_epoch(value: str | None) -> float | None:
    if not value:
        return None
    try:
        fixed = value.replace("Z", "+00:00")
        return dt.datetime.fromisoformat(fixed).timestamp()
    except ValueError:
        return None


def load_tasks(repo_dir: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    tasks_dir = repo_dir / ".tak" / "tasks"
    tasks: list[dict[str, Any]] = []
    normalization_issues: list[dict[str, Any]] = []

    if not tasks_dir.exists():
        return tasks, normalization_issues

    for task_path in sorted(tasks_dir.glob("*.json"), key=lambda p: p.name):
        try:
            task = json.loads(task_path.read_text(encoding="utf-8"))
            tasks.append(task)
        except json.JSONDecodeError:
            normalization_issues.append(
                {
                    "file": str(task_path.relative_to(repo_dir)),
                    "issue": "invalid_json",
                }
            )
            continue

        tags = task.get("tags", [])
        if isinstance(tags, list) and tags != sorted(set(tags)):
            normalization_issues.append(
                {
                    "file": str(task_path.relative_to(repo_dir)),
                    "issue": "tags_not_sorted_unique",
                }
            )

        depends_on = task.get("depends_on", [])
        dep_ids: list[int] = []
        if isinstance(depends_on, list):
            for dep in depends_on:
                dep_id: int | None = None
                if isinstance(dep, dict):
                    raw_id = dep.get("id")
                    if isinstance(raw_id, int):
                        dep_id = raw_id
                elif isinstance(dep, int):
                    dep_id = dep
                if dep_id is not None:
                    dep_ids.append(dep_id)

        if dep_ids and dep_ids != sorted(set(dep_ids)):
            normalization_issues.append(
                {
                    "file": str(task_path.relative_to(repo_dir)),
                    "issue": "depends_on_not_sorted_unique",
                }
            )

        title = task.get("title")
        if isinstance(title, str) and title != title.strip():
            normalization_issues.append(
                {
                    "file": str(task_path.relative_to(repo_dir)),
                    "issue": "title_not_trimmed",
                }
            )

    return tasks, normalization_issues


def collect_tak_metrics(repo_dir: Path, change_epoch: float | None) -> dict[str, Any]:
    tasks, normalization_issues = load_tasks(repo_dir)

    history_dir = repo_dir / ".tak" / "history"
    verification_dir = repo_dir / ".tak" / "verification_results"
    context_dir = repo_dir / ".tak" / "context"
    learnings_dir = repo_dir / ".tak" / "learnings"

    status_counts: dict[str, int] = {}
    kind_counts: dict[str, int] = {}
    dependency_edges = 0
    child_count = 0
    epic_count = 0
    contract_task_count = 0
    contract_verification_task_count = 0

    for task in tasks:
        status = str(task.get("status", "unknown"))
        status_counts[status] = status_counts.get(status, 0) + 1

        kind = str(task.get("kind", "unknown"))
        kind_counts[kind] = kind_counts.get(kind, 0) + 1
        if kind == "epic":
            epic_count += 1

        parent_id = task.get("parent_id")
        if isinstance(parent_id, int):
            child_count += 1

        depends_on = task.get("depends_on", [])
        if isinstance(depends_on, list):
            dependency_edges += len(depends_on)

        contract = task.get("contract", {})
        has_contract = False
        has_verification = False
        if isinstance(contract, dict):
            objective = contract.get("objective")
            criteria = contract.get("acceptance_criteria")
            verify = contract.get("verification")
            constraints = contract.get("constraints")

            if any(
                [
                    bool(objective),
                    bool(criteria),
                    bool(verify),
                    bool(constraints),
                ]
            ):
                has_contract = True
            if isinstance(verify, list) and len(verify) > 0:
                has_verification = True

        if has_contract:
            contract_task_count += 1
        if has_verification:
            contract_verification_task_count += 1

    history_event_count = 0
    history_event_counts: dict[str, int] = {}
    history_events_after_change = 0

    if history_dir.exists():
        for history_path in history_dir.glob("*.jsonl"):
            lines = history_path.read_text(encoding="utf-8", errors="replace").splitlines()
            for line in lines:
                if not line.strip():
                    continue
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                history_event_count += 1
                event_name = str(event.get("event", "unknown"))
                history_event_counts[event_name] = history_event_counts.get(event_name, 0) + 1

                if change_epoch is not None:
                    event_epoch = parse_iso_epoch(event.get("timestamp"))
                    if event_epoch is not None and event_epoch >= change_epoch:
                        history_events_after_change += 1

    verification_result_count = len(list(verification_dir.glob("*.json"))) if verification_dir.exists() else 0
    context_count = len(list(context_dir.glob("*.md"))) if context_dir.exists() else 0

    learning_count = 0
    if learnings_dir.exists():
        learning_count = len(
            [
                p
                for p in learnings_dir.glob("*.json")
                if p.name != "counter.json"
            ]
        )

    tasks_modified_after_change = 0
    if change_epoch is not None:
        tasks_dir = repo_dir / ".tak" / "tasks"
        if tasks_dir.exists():
            for path in tasks_dir.glob("*.json"):
                if path.stat().st_mtime >= change_epoch:
                    tasks_modified_after_change += 1

    return {
        "task_count": len(tasks),
        "status_counts": status_counts,
        "kind_counts": kind_counts,
        "epic_count": epic_count,
        "child_count": child_count,
        "dependency_edges": dependency_edges,
        "contract_task_count": contract_task_count,
        "contract_verification_task_count": contract_verification_task_count,
        "history_event_count": history_event_count,
        "history_event_counts": history_event_counts,
        "history_events_after_change": history_events_after_change,
        "verification_result_count": verification_result_count,
        "context_count": context_count,
        "learning_count": learning_count,
        "tasks_modified_after_change": tasks_modified_after_change,
        "normalization_issues": normalization_issues,
    }


def collect_git_metrics(repo_dir: Path, baseline_sha: str, change_epoch: float | None) -> dict[str, Any]:
    rev_list = run_cmd(["git", "rev-list", "--reverse", f"{baseline_sha}..HEAD"], cwd=repo_dir)
    ensure_success(rev_list, "git rev-list")

    shas = [line.strip() for line in rev_list.stdout.splitlines() if line.strip()]
    commits: list[dict[str, Any]] = []

    for sha in shas:
        meta = run_cmd(
            ["git", "show", "-s", "--format=%H%x1f%s%x1f%ct", sha],
            cwd=repo_dir,
        )
        ensure_success(meta, f"git show meta for {sha}")
        meta_parts = meta.stdout.strip().split("\x1f")
        if len(meta_parts) < 3:
            continue

        files_result = run_cmd(
            ["git", "show", "--name-only", "--pretty=format:", sha],
            cwd=repo_dir,
        )
        ensure_success(files_result, f"git show files for {sha}")
        files = [line.strip() for line in files_result.stdout.splitlines() if line.strip()]

        numstat_result = run_cmd(
            ["git", "show", "--numstat", "--pretty=format:", sha],
            cwd=repo_dir,
        )
        ensure_success(numstat_result, f"git show numstat for {sha}")

        lines_changed = 0
        for line in numstat_result.stdout.splitlines():
            parts = line.split("\t")
            if len(parts) < 3:
                continue
            if parts[0].isdigit() and parts[1].isdigit():
                lines_changed += int(parts[0]) + int(parts[1])

        commits.append(
            {
                "sha": meta_parts[0],
                "message": meta_parts[1],
                "timestamp": int(meta_parts[2]),
                "files": files,
                "file_count": len(files),
                "lines_changed": lines_changed,
            }
        )

    commit_count = len(commits)
    first_tak_index = None
    first_code_index = None

    for i, commit in enumerate(commits):
        files = commit["files"]
        if first_tak_index is None and any(path.startswith(".tak/") for path in files):
            first_tak_index = i
        if first_code_index is None and any(not path.startswith(".tak/") for path in files):
            first_code_index = i

    small_commits = 0
    good_messages = 0
    weak_message_re = re.compile(r"(?i)^(wip|update|fix|changes|misc|tmp)\b")
    total_lines_changed = 0

    for commit in commits:
        if 0 < commit["file_count"] <= 3:
            small_commits += 1

        message = commit["message"].strip()
        if len(message) >= 10 and not weak_message_re.search(message):
            good_messages += 1

        total_lines_changed += commit["lines_changed"]

    small_commit_ratio = (small_commits / commit_count) if commit_count > 0 else 0.0
    good_message_ratio = (good_messages / commit_count) if commit_count > 0 else 0.0

    final_commit_ratio = 0.0
    if commit_count > 0 and total_lines_changed > 0:
        final_commit_ratio = commits[-1]["lines_changed"] / total_lines_changed

    commits_after_change = 0
    if change_epoch is not None:
        commits_after_change = len(
            [commit for commit in commits if commit["timestamp"] >= int(change_epoch)]
        )

    return {
        "commit_count": commit_count,
        "commits": commits,
        "first_tak_commit_index": first_tak_index,
        "first_code_commit_index": first_code_index,
        "small_commit_ratio": small_commit_ratio,
        "good_message_ratio": good_message_ratio,
        "final_commit_ratio": final_commit_ratio,
        "commits_after_change": commits_after_change,
        "total_lines_changed": total_lines_changed,
    }


def transcript_metrics(
    worker_log_text: str,
    command_events: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    raw_tak_mentions = len(re.findall(r"\btak\b", worker_log_text))
    raw_tak_verify_mentions = len(re.findall(r"\btak\s+verify\b", worker_log_text))
    raw_pytest_mentions = len(re.findall(r"\bpytest\b", worker_log_text))

    prompt_prefix_re = re.compile(r"^[^\n]{0,120}?(?:\$|#|%|❯|➜)\s+")
    command_start_re = re.compile(
        r"^(?:[\w./-]*/)?(?:tak|git|pytest|python(?:\d+(?:\.\d+)*)?|uv|bash|sh|ls|cat|grep|rg|sed|awk|perl|jq|tee|vim|nvim|nano|vi|cp|mv|echo|touch|mkdir|rm|find|fd|cargo|make|npm|pnpm|yarn)(?:\s|$)"
    )

    tak_cmd_re = re.compile(r"^(?:[\w./-]*/)?tak(?:\s|$)")
    tak_verify_cmd_re = re.compile(r"^(?:[\w./-]*/)?tak\s+verify(?:\s|$)")
    pytest_cmd_re = re.compile(
        r"^(?:pytest(?:\s|$)|(?:[\w./-]*/)?python(?:\d+(?:\.\d+)*)?\s+-m\s+pytest(?:\s|$)|uv\s+run\b.*\bpytest(?:\s|$))"
    )

    command_like_lines: list[str] = []
    extracted_commands: list[str] = []

    for line in worker_log_text.splitlines():
        stripped = line.strip()
        if not stripped:
            continue

        normalized = prompt_prefix_re.sub("", stripped, count=1)
        if len(normalized) > 260:
            continue

        if command_start_re.search(normalized):
            command_like_lines.append(stripped)
            extracted_commands.append(normalized)

    tak_command_count = 0
    tak_verify_command_count = 0
    pytest_command_count = 0
    test_command_count = 0

    for command in extracted_commands:
        if tak_cmd_re.search(command):
            tak_command_count += 1
        if tak_verify_cmd_re.search(command):
            tak_verify_command_count += 1
        if pytest_cmd_re.search(command):
            pytest_command_count += 1
        if tak_verify_cmd_re.search(command) or pytest_cmd_re.search(command):
            test_command_count += 1

    evidence: list[str] = []

    editor_re = re.compile(
        r"(?i)\b(?:vim|nvim|nano|vi)\b[^\n]*\.tak/(?:tasks|learnings|history|counter|config)",
    )
    redirection_re = re.compile(
        r"(?i)(?:>|>>)\s*\.tak/(?:tasks|learnings|history|counter|config)",
    )
    inplace_re = re.compile(
        r"(?i)\b(?:sed|perl)\b[^\n]*-i[^\n]*\.tak/(?:tasks|learnings|history|counter|config)",
    )
    copy_re = re.compile(
        r"(?i)\b(?:cp|mv)\b[^\n]*\.tak/(?:tasks|learnings|history|counter|config)",
    )

    for line in extracted_commands:
        if (
            editor_re.search(line)
            or redirection_re.search(line)
            or inplace_re.search(line)
            or copy_re.search(line)
        ):
            evidence.append(line)

    # Keep deterministic and concise evidence samples.
    deduped = []
    seen = set()
    for line in evidence:
        if line not in seen:
            seen.add(line)
            deduped.append(line)

    command_events = command_events or []
    event_type_counts: dict[str, int] = {}
    for event in command_events:
        event_type = str(event.get("type", ""))
        if not event_type:
            continue
        event_type_counts[event_type] = event_type_counts.get(event_type, 0) + 1

    return {
        # Command-extracted (primary)
        "tak_mentions": tak_command_count,
        "tak_verify_mentions": tak_verify_command_count,
        "pytest_mentions": pytest_command_count,
        "test_command_mentions": test_command_count,
        "extracted_command_count": len(extracted_commands),
        "extracted_command_sample": extracted_commands[:20],
        # Raw text (secondary diagnostics)
        "raw_tak_mentions": raw_tak_mentions,
        "raw_tak_verify_mentions": raw_tak_verify_mentions,
        "raw_pytest_mentions": raw_pytest_mentions,
        "command_like_line_count": len(command_like_lines),
        "manual_tak_edit_evidence": deduped,
        "manual_tak_edit_command_count": len(deduped),
        "harness_event_type_counts": event_type_counts,
    }


def run_scoring(
    *,
    objective: ObjectiveConfig,
    public_result: ExecResult,
    hidden_result: ExecResult | None,
    change_probe_result: ExecResult | None,
    tak: dict[str, Any],
    git: dict[str, Any],
    transcript: dict[str, Any],
    change_injected: bool,
    change_injected_epoch: float | None,
) -> dict[str, Any]:
    scoring = objective.scoring

    w_public = scoring.get("functional_public", 15)
    w_hidden = scoring.get("functional_hidden", 30)
    w_tak = scoring.get("tak_workflow", 25)
    w_git = scoring.get("git_discipline", 20)
    w_change = scoring.get("change_adaptation", 10)
    w_bonus_cap = scoring.get("bonus", 10)

    # Functional
    public_pass = public_result.exit_code == 0
    hidden_pass = (hidden_result.exit_code == 0) if hidden_result is not None else False
    change_probe_pass = (change_probe_result.exit_code == 0) if change_probe_result else False

    functional_score = (w_public if public_pass else 0) + (w_hidden if hidden_pass else 0)

    # tak workflow (25)
    tak_components: dict[str, int] = {
        "early_adoption": 0,
        "planning_structure": 0,
        "lifecycle_hygiene": 0,
        "verification_discipline": 0,
    }

    first_tak = git.get("first_tak_commit_index")
    first_code = git.get("first_code_commit_index")
    if first_tak is not None and (first_code is None or first_tak <= first_code):
        tak_components["early_adoption"] = 5
    elif first_tak is not None and first_code is not None and (first_tak - first_code) <= 1:
        tak_components["early_adoption"] = 3
    elif tak.get("task_count", 0) > 0:
        tak_components["early_adoption"] = 1

    task_count = tak.get("task_count", 0)
    epic_count = tak.get("epic_count", 0)
    child_count = tak.get("child_count", 0)
    dep_edges = tak.get("dependency_edges", 0)

    if epic_count >= 1 and task_count >= 5 and child_count >= 3:
        tak_components["planning_structure"] = 8
    elif task_count >= 4 and dep_edges >= 1:
        tak_components["planning_structure"] = 6
    elif task_count >= 2:
        tak_components["planning_structure"] = 3
    elif task_count >= 1:
        tak_components["planning_structure"] = 1

    lifecycle_score = 8
    in_progress = tak.get("status_counts", {}).get("in_progress", 0)
    done_count = tak.get("status_counts", {}).get("done", 0)
    history_counts = tak.get("history_event_counts", {})

    if in_progress > 0:
        lifecycle_score -= min(4, in_progress)
    if history_counts.get("start", 0) == 0 and task_count > 0:
        lifecycle_score -= 2
    if done_count > 0 and history_counts.get("finish", 0) == 0:
        lifecycle_score -= 2
    lifecycle_score = max(0, lifecycle_score)
    tak_components["lifecycle_hygiene"] = lifecycle_score

    verify_mentions = transcript.get("tak_verify_mentions", 0)
    verification_files = tak.get("verification_result_count", 0)
    pytest_mentions = transcript.get("pytest_mentions", 0)
    test_command_mentions = transcript.get("test_command_mentions", 0)

    if verify_mentions > 0 and verification_files > 0:
        tak_components["verification_discipline"] = 4
    elif test_command_mentions >= 2:
        tak_components["verification_discipline"] = 2
    elif test_command_mentions >= 1 or pytest_mentions >= 1:
        tak_components["verification_discipline"] = 1

    tak_workflow_score = sum(tak_components.values())
    tak_workflow_score = min(w_tak, tak_workflow_score)

    # Git discipline (20)
    git_components: dict[str, int] = {
        "cadence": 0,
        "atomicity": 0,
        "message_quality": 0,
    }
    commit_count = git.get("commit_count", 0)

    if commit_count >= 5:
        git_components["cadence"] = 8
    elif commit_count >= 4:
        git_components["cadence"] = 6
    elif commit_count >= 3:
        git_components["cadence"] = 4
    elif commit_count >= 1:
        git_components["cadence"] = 2

    small_ratio = git.get("small_commit_ratio", 0.0)
    if small_ratio >= 0.7:
        git_components["atomicity"] = 8
    elif small_ratio >= 0.5:
        git_components["atomicity"] = 6
    elif small_ratio >= 0.3:
        git_components["atomicity"] = 4
    elif small_ratio > 0:
        git_components["atomicity"] = 2

    msg_ratio = git.get("good_message_ratio", 0.0)
    if msg_ratio >= 0.75:
        git_components["message_quality"] = 4
    elif msg_ratio >= 0.5:
        git_components["message_quality"] = 3
    elif msg_ratio >= 0.25:
        git_components["message_quality"] = 2
    elif msg_ratio > 0:
        git_components["message_quality"] = 1

    git_score = min(w_git, sum(git_components.values()))

    # Change adaptation (10)
    change_components: dict[str, int] = {
        "replan_after_change": 0,
        "implement_and_validate_change": 0,
        "close_loop": 0,
    }

    if change_injected:
        tasks_after_change = tak.get("tasks_modified_after_change", 0)
        history_after_change = tak.get("history_events_after_change", 0)

        if tasks_after_change > 0 and history_after_change > 0:
            change_components["replan_after_change"] = 4
        elif tasks_after_change > 0:
            change_components["replan_after_change"] = 2

        if change_probe_pass and hidden_pass:
            change_components["implement_and_validate_change"] = 4
        elif change_probe_pass:
            change_components["implement_and_validate_change"] = 3
        elif hidden_pass:
            change_components["implement_and_validate_change"] = 2

        commits_after_change = git.get("commits_after_change", 0)
        in_progress_after = tak.get("status_counts", {}).get("in_progress", 0)
        if commits_after_change >= 1 and in_progress_after == 0:
            change_components["close_loop"] = 2
        elif commits_after_change >= 1:
            change_components["close_loop"] = 1

    change_score = min(w_change, sum(change_components.values()))

    # Bonus (10)
    bonus_components: dict[str, int] = {
        "contract_richness": 0,
        "verify_usage": 0,
        "context_or_learnings": 0,
    }

    contract_tasks = tak.get("contract_task_count", 0)
    contract_verify_tasks = tak.get("contract_verification_task_count", 0)
    if contract_tasks >= 2 and contract_verify_tasks >= 1:
        bonus_components["contract_richness"] = 4
    elif contract_tasks >= 1:
        bonus_components["contract_richness"] = 2

    if verify_mentions > 0 and verification_files > 0:
        bonus_components["verify_usage"] = 3
    elif verify_mentions > 0:
        bonus_components["verify_usage"] = 1

    context_count = tak.get("context_count", 0)
    learning_count = tak.get("learning_count", 0)
    if context_count > 0 and learning_count > 0:
        bonus_components["context_or_learnings"] = 3
    elif context_count > 0 or learning_count > 0:
        bonus_components["context_or_learnings"] = 2

    bonus_score = min(w_bonus_cap, sum(bonus_components.values()))

    # Penalties
    penalties: dict[str, int] = {
        "manual_tak_edits": 0,
        "no_commits": 0,
        "few_commits": 0,
        "giant_final_commit": 0,
    }

    incident_count = len(transcript.get("manual_tak_edit_evidence", [])) + len(
        tak.get("normalization_issues", [])
    )

    first_penalty = scoring.get("penalty_manual_tak_first", 20)
    add_penalty = scoring.get("penalty_manual_tak_additional", 10)
    cap_penalty = scoring.get("penalty_manual_tak_cap", 40)

    if incident_count > 0:
        total = first_penalty + max(0, incident_count - 1) * add_penalty
        penalties["manual_tak_edits"] = min(cap_penalty, total)

    if commit_count == 0:
        penalties["no_commits"] = 30
        git_score = 0
    elif commit_count < 4:
        penalties["few_commits"] = 10

    if git.get("final_commit_ratio", 0.0) > 0.6 and commit_count >= 1:
        penalties["giant_final_commit"] = 8

    penalties_total = sum(penalties.values())

    core_total = functional_score + tak_workflow_score + git_score + change_score
    total_raw = core_total + bonus_score - penalties_total
    total_clamped = max(0, total_raw)

    return {
        "functional": {
            "score": functional_score,
            "max": w_public + w_hidden,
            "public_pass": public_pass,
            "hidden_pass": hidden_pass,
            "change_probe_pass": change_probe_pass,
        },
        "tak_workflow": {
            "score": tak_workflow_score,
            "max": w_tak,
            "components": tak_components,
        },
        "git_discipline": {
            "score": git_score,
            "max": w_git,
            "components": git_components,
        },
        "change_adaptation": {
            "score": change_score,
            "max": w_change,
            "components": change_components,
            "change_injected": change_injected,
            "change_injected_epoch": change_injected_epoch,
        },
        "bonus": {
            "score": bonus_score,
            "max": w_bonus_cap,
            "components": bonus_components,
        },
        "penalties": {
            "score": penalties_total,
            "components": penalties,
            "manual_tak_edit_incidents": incident_count,
        },
        "totals": {
            "core": core_total,
            "bonus": bonus_score,
            "penalties": penalties_total,
            "raw": total_raw,
            "clamped": total_clamped,
        },
    }


def drive_worker_session(
    *,
    objective: ObjectiveConfig,
    repo_dir: Path,
    pane_target: str,
    worker_log_path: Path,
    commands_log_path: Path,
    initial_prompt: str,
    change_prompt: str,
    worker_cmd: str,
) -> dict[str, Any]:
    tmux_send_line(pane_target, f"cd {shlex.quote(str(repo_dir))}", commands_log_path, "setup")
    tmux_send_line(
        pane_target,
        f"export TAKBENCH_OBJECTIVE={shlex.quote(objective.objective_id)}",
        commands_log_path,
        "setup",
    )
    tmux_send_line(
        pane_target,
        f"export TAKBENCH_REPO={shlex.quote(str(repo_dir))}",
        commands_log_path,
        "setup",
    )

    if objective.worker_prompt_transport != "tmux_buffer_paste":
        raise RuntimeError(
            "Unsupported worker.prompt_transport in objective manifest: "
            f"{objective.worker_prompt_transport}"
        )

    tmux_send_line(pane_target, worker_cmd, commands_log_path, "worker_start")

    ready_state = wait_for_worker_ready(objective, worker_log_path)
    append_jsonl(
        commands_log_path,
        {
            "timestamp": utc_now(),
            "type": "worker_ready",
            "target": pane_target,
            "prompt_transport": objective.worker_prompt_transport,
            **ready_state,
        },
    )

    tmux_send_prompt(pane_target, initial_prompt, commands_log_path, "initial_prompt")

    start_epoch = time.time()
    min_change_seconds = objective.change_min_minutes * 60
    target_change_seconds = objective.change_target_minutes * 60
    budget_seconds = objective.time_budget_minutes * 60

    next_public_probe_at = start_epoch + objective.public_probe_interval_seconds

    change_injected = False
    change_injected_epoch: float | None = None
    phase1_done_seen = False
    final_done_seen = False
    public_probe_pass = False
    probe_results: list[dict[str, Any]] = []

    end_reason = "timeout"

    while True:
        now = time.time()
        elapsed = now - start_epoch

        worker_text = read_text_safe(worker_log_path)

        if objective.phase1_done_token in worker_text:
            phase1_done_seen = True
        if objective.final_done_token in worker_text:
            final_done_seen = True

        if now >= next_public_probe_at:
            probe_result = exec_shell(objective.public_test_command, repo_dir, timeout=180)
            probe_pass = probe_result.exit_code == 0
            if probe_pass:
                public_probe_pass = True
            probe_event = {
                "timestamp": utc_now(),
                "type": "public_probe",
                "exit_code": probe_result.exit_code,
                "passed": probe_pass,
            }
            probe_results.append(probe_event)
            append_jsonl(commands_log_path, probe_event)
            next_public_probe_at = now + objective.public_probe_interval_seconds

        change_trigger = (
            elapsed >= min_change_seconds
            and (
                phase1_done_seen
                or public_probe_pass
                or elapsed >= target_change_seconds
            )
        )

        if change_trigger and not change_injected:
            tmux_send_prompt(pane_target, change_prompt, commands_log_path, "change_prompt")
            change_injected = True
            change_injected_epoch = time.time()

        if pane_dead(pane_target):
            end_reason = "worker_pane_dead"
            break

        if change_injected and final_done_seen:
            end_reason = "final_done_token_seen"
            break

        if elapsed >= budget_seconds:
            end_reason = "time_budget_reached"
            break

        time.sleep(max(1, objective.poll_interval_seconds))

    return {
        "start_epoch": start_epoch,
        "end_epoch": time.time(),
        "duration_seconds": time.time() - start_epoch,
        "end_reason": end_reason,
        "worker_ready": ready_state.get("ready"),
        "worker_ready_strategy": ready_state.get("strategy"),
        "worker_ready_wait_seconds": ready_state.get("wait_seconds"),
        "worker_ready_reason": ready_state.get("reason"),
        "change_injected": change_injected,
        "change_injected_epoch": change_injected_epoch,
        "phase1_done_seen": phase1_done_seen,
        "final_done_seen": final_done_seen,
        "public_probe_pass": public_probe_pass,
        "probe_results": probe_results,
    }


def run_tests_and_probes(
    *,
    objective: ObjectiveConfig,
    repo_dir: Path,
    logs_dir: Path,
    hidden_test_command_override: str | None,
) -> dict[str, Any]:
    public_result = exec_shell(objective.public_test_command, repo_dir, timeout=300)
    save_exec_log(logs_dir / "public-tests.log", "Public tests", public_result)

    hidden_command = hidden_test_command_override or objective.hidden_test_command
    hidden_result: ExecResult | None = None
    if hidden_command:
        hidden_result = exec_shell(hidden_command, repo_dir, timeout=300)
        save_exec_log(logs_dir / "hidden-tests.log", "Hidden tests", hidden_result)

    change_probe_result: ExecResult | None = None
    if objective.change_probe_command:
        change_probe_result = exec_shell(objective.change_probe_command, repo_dir, timeout=120)
        save_exec_log(logs_dir / "change-probe.log", "Change probe", change_probe_result)

    return {
        "public_result": public_result,
        "hidden_result": hidden_result,
        "change_probe_result": change_probe_result,
        "hidden_command": hidden_command,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="takbench draft harness")
    parser.add_argument(
        "--objective",
        type=Path,
        default=Path("bench/objectives/markdown_parser_v1"),
        help="Objective pack directory or objective.toml path",
    )
    parser.add_argument(
        "--worker-cmd",
        required=True,
        help="Command to launch the interactive puppet agent in tmux",
    )
    parser.add_argument(
        "--runs-dir",
        type=Path,
        default=Path("bench/runs"),
        help="Directory for benchmark run artifacts",
    )
    parser.add_argument(
        "--run-id",
        type=str,
        default="",
        help="Optional run id (default: generated timestamp)",
    )
    parser.add_argument(
        "--hidden-test-cmd",
        type=str,
        default="",
        help="Optional override hidden test command",
    )
    parser.add_argument(
        "--time-budget-minutes",
        type=int,
        default=0,
        help="Optional override for objective time budget",
    )
    parser.add_argument(
        "--session-prefix",
        type=str,
        default="takbench",
        help="tmux session name prefix",
    )
    parser.add_argument(
        "--skip-tak-init",
        action="store_true",
        help="Do not run tak init during repo scaffolding",
    )
    parser.add_argument(
        "--allow-missing-hidden-tests",
        action="store_true",
        help="Allow run without hidden test command even if objective requires it",
    )
    parser.add_argument(
        "--keep-tmux-session",
        action="store_true",
        help="Do not kill tmux session at run end",
    )
    parser.add_argument(
        "--allow-non-uv",
        action="store_true",
        help="Allow running outside uv-managed Python runtime",
    )

    args = parser.parse_args()

    objective = load_objective(args.objective)
    if args.time_budget_minutes > 0:
        objective.time_budget_minutes = args.time_budget_minutes

    bench_dir = Path(__file__).resolve().parent
    if not args.allow_non_uv and not running_in_uv_runtime(bench_dir):
        print(
            "Error: takbench must run in a uv-managed environment.\n"
            "Use: uv sync --project bench && uv run --project bench python bench/takbench.py ...\n"
            "(or pass --allow-non-uv to bypass this guard)",
            file=sys.stderr,
        )
        return 2

    hidden_cmd_override = args.hidden_test_cmd.strip() or None
    effective_hidden_cmd = hidden_cmd_override or objective.hidden_test_command
    if objective.hidden_tests_required and not effective_hidden_cmd and not args.allow_missing_hidden_tests:
        print(
            "Error: objective requires hidden tests but no hidden command is configured. "
            "Set --hidden-test-cmd or hidden_test_command in objective.toml.",
            file=sys.stderr,
        )
        return 2

    pytest_required = (
        "pytest" in objective.public_test_command
        or "pytest" in objective.change_probe_command
        or (bool(effective_hidden_cmd) and "pytest" in effective_hidden_cmd)
    )
    if pytest_required:
        try:
            ensure_python_module(
                "pytest",
                "Run `uv sync --project bench`, then invoke via `uv run --project bench python bench/takbench.py ...`",
            )
        except RuntimeError as exc:
            print(f"Error: {exc}", file=sys.stderr)
            return 2

    require_binary("tmux")
    require_binary("git")
    if not args.skip_tak_init:
        require_binary("tak")

    run_id = args.run_id.strip()
    if not run_id:
        run_id = f"{objective.objective_id}_{dt.datetime.now().strftime('%Y%m%d_%H%M%S')}"

    paths = setup_run_dirs(args.runs_dir, run_id)
    run_dir = paths["run_dir"]
    repo_dir = paths["repo_dir"]
    logs_dir = paths["logs_dir"]
    prompts_dir = paths["prompts_dir"]

    initial_prompt = objective.initial_prompt_path.read_text(encoding="utf-8")
    change_prompt = objective.change_prompt_path.read_text(encoding="utf-8")

    (prompts_dir / "initial.txt").write_text(initial_prompt, encoding="utf-8")
    (prompts_dir / "change.txt").write_text(change_prompt, encoding="utf-8")

    baseline_sha = prepare_repo(
        repo_dir=repo_dir,
        template_dir=objective.template_dir,
        init_tak=not args.skip_tak_init,
    )

    session_name = sanitize_session_name(f"{args.session_prefix}_{run_id}")
    pane_log_path = logs_dir / "worker.log"
    pane_capture_path = logs_dir / "pane-final.txt"
    commands_log_path = logs_dir / "commands.jsonl"
    tmux_meta_path = run_dir / "tmux_meta.json"

    pane_target = start_tmux_session(session_name, pane_log_path)

    tmux_meta: dict[str, Any] = {
        "session": session_name,
        "pane": pane_target,
        "started_at": utc_now(),
        "worker_cmd": args.worker_cmd,
    }
    tmux_meta_path.write_text(json.dumps(tmux_meta, indent=2), encoding="utf-8")

    session_result: dict[str, Any] | None = None
    try:
        session_result = drive_worker_session(
            objective=objective,
            repo_dir=repo_dir,
            pane_target=pane_target,
            worker_log_path=pane_log_path,
            commands_log_path=commands_log_path,
            initial_prompt=initial_prompt,
            change_prompt=change_prompt,
            worker_cmd=args.worker_cmd,
        )

        # Allow the worker process to flush before capture.
        time.sleep(1)
        capture_pane(pane_target, pane_capture_path)

    finally:
        tmux_meta["ended_at"] = utc_now()
        if session_result is not None:
            tmux_meta["end_reason"] = session_result.get("end_reason")
            tmux_meta["duration_seconds"] = session_result.get("duration_seconds")
            tmux_meta["change_injected"] = session_result.get("change_injected")
            tmux_meta["change_injected_epoch"] = session_result.get("change_injected_epoch")
            tmux_meta["worker_ready"] = session_result.get("worker_ready")
            tmux_meta["worker_ready_strategy"] = session_result.get("worker_ready_strategy")
            tmux_meta["worker_ready_reason"] = session_result.get("worker_ready_reason")
            tmux_meta["worker_ready_wait_seconds"] = session_result.get("worker_ready_wait_seconds")
            tmux_meta["phase1_done_seen"] = session_result.get("phase1_done_seen")
            tmux_meta["final_done_seen"] = session_result.get("final_done_seen")
        tmux_meta_path.write_text(json.dumps(tmux_meta, indent=2), encoding="utf-8")

        if not args.keep_tmux_session:
            kill_session(session_name)

    test_outputs = run_tests_and_probes(
        objective=objective,
        repo_dir=repo_dir,
        logs_dir=logs_dir,
        hidden_test_command_override=hidden_cmd_override,
    )

    worker_log_text = read_text_safe(pane_log_path)
    command_events = load_jsonl(commands_log_path)

    change_epoch = None
    if session_result is not None:
        change_epoch = session_result.get("change_injected_epoch")

    tak_metrics = collect_tak_metrics(repo_dir, change_epoch)
    git_metrics = collect_git_metrics(repo_dir, baseline_sha, change_epoch)
    transcript = transcript_metrics(worker_log_text, command_events)

    scoring = run_scoring(
        objective=objective,
        public_result=test_outputs["public_result"],
        hidden_result=test_outputs["hidden_result"],
        change_probe_result=test_outputs["change_probe_result"],
        tak=tak_metrics,
        git=git_metrics,
        transcript=transcript,
        change_injected=bool(session_result and session_result.get("change_injected")),
        change_injected_epoch=change_epoch,
    )
    command_event_types = [str(event.get("type", "")) for event in command_events]
    command_event_type_set = set(command_event_types)

    validity_checks: dict[str, Any] = {}
    invalid_reasons: list[str] = []

    worker_log_present = pane_log_path.exists() and pane_log_path.stat().st_size > 0
    commands_log_present = commands_log_path.exists() and commands_log_path.stat().st_size > 0
    public_test_executed = test_command_executed(test_outputs["public_result"])
    hidden_test_executed = test_command_executed(test_outputs["hidden_result"])

    validity_checks["worker_log_present"] = worker_log_present
    validity_checks["commands_log_present"] = commands_log_present
    validity_checks["session_result_present"] = session_result is not None
    validity_checks["worker_start_event_present"] = "worker_start" in command_event_type_set
    validity_checks["worker_ready_event_present"] = "worker_ready" in command_event_type_set
    validity_checks["initial_prompt_event_present"] = "initial_prompt" in command_event_type_set
    validity_checks["public_test_executed"] = public_test_executed
    validity_checks["hidden_test_executed"] = hidden_test_executed
    validity_checks["worker_ready_success"] = bool(
        session_result and session_result.get("worker_ready") is True
    )

    if not worker_log_present:
        invalid_reasons.append("missing_or_empty_worker_log")
    if not commands_log_present:
        invalid_reasons.append("missing_or_empty_commands_log")
    if session_result is None:
        invalid_reasons.append("missing_session_result")
    if "worker_start" not in command_event_type_set:
        invalid_reasons.append("missing_worker_start_event")
    if "initial_prompt" not in command_event_type_set:
        invalid_reasons.append("missing_initial_prompt_event")
    if "worker_ready" not in command_event_type_set:
        invalid_reasons.append("missing_worker_ready_event")
    if session_result and session_result.get("worker_ready") is False:
        invalid_reasons.append("worker_ready_check_failed")
    if not public_test_executed:
        invalid_reasons.append("public_tests_not_executed")

    if objective.hidden_tests_required and not test_outputs["hidden_command"] and not args.allow_missing_hidden_tests:
        invalid_reasons.append("missing_hidden_test_command")

    if objective.hidden_tests_required and not args.allow_missing_hidden_tests and not hidden_test_executed:
        invalid_reasons.append("hidden_tests_not_executed")

    if session_result and session_result.get("change_injected"):
        change_prompt_injected = "change_prompt" in command_event_type_set
        validity_checks["change_prompt_event_present"] = change_prompt_injected
        if not change_prompt_injected:
            invalid_reasons.append("missing_change_prompt_event")

    worker_progress_evidence = (
        bool(session_result and (session_result.get("phase1_done_seen") or session_result.get("final_done_seen")))
        or git_metrics.get("commit_count", 0) > 0
        or tak_metrics.get("task_count", 0) > 0
        or tak_metrics.get("history_event_count", 0) > 0
    )
    validity_checks["worker_progress_evidence"] = worker_progress_evidence
    if not worker_progress_evidence:
        invalid_reasons.append("no_worker_progress_evidence")

    report = {
        "run_id": run_id,
        "objective": {
            "id": objective.objective_id,
            "name": objective.name,
            "description": objective.description,
            "time_budget_minutes": objective.time_budget_minutes,
            "manifest_root": str(objective.root),
            "worker_protocol": {
                "ready_strategy": objective.worker_ready_strategy,
                "ready_delay_seconds": objective.worker_ready_delay_seconds,
                "ready_timeout_seconds": objective.worker_ready_timeout_seconds,
                "ready_token": objective.worker_ready_token,
                "prompt_transport": objective.worker_prompt_transport,
                "phase1_done_token": objective.phase1_done_token,
                "final_done_token": objective.final_done_token,
            },
        },
        "runtime": {
            "python_executable": sys.executable,
            "uv_runtime_detected": running_in_uv_runtime(bench_dir),
            "allow_non_uv": args.allow_non_uv,
        },
        "timestamps": {
            "created_at": utc_now(),
            "session": session_result,
        },
        "paths": {
            "run_dir": str(run_dir),
            "repo_dir": str(repo_dir),
            "worker_log": str(pane_log_path),
            "commands_log": str(commands_log_path),
            "pane_capture": str(pane_capture_path),
            "tmux_meta": str(tmux_meta_path),
        },
        "baseline_sha": baseline_sha,
        "tests": {
            "public": dataclasses.asdict(test_outputs["public_result"]),
            "hidden": dataclasses.asdict(test_outputs["hidden_result"]) if test_outputs["hidden_result"] else None,
            "change_probe": dataclasses.asdict(test_outputs["change_probe_result"]) if test_outputs["change_probe_result"] else None,
            "hidden_command": test_outputs["hidden_command"],
        },
        "metrics": {
            "tak": tak_metrics,
            "git": git_metrics,
            "transcript": transcript,
        },
        "scores": scoring,
        "validity": {
            "checks": validity_checks,
            "command_event_count": len(command_events),
            "command_event_types": sorted(command_event_type_set),
        },
        "valid": len(invalid_reasons) == 0,
        "invalid_reasons": invalid_reasons,
    }

    report_path = run_dir / "report.json"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")

    total = scoring["totals"]["clamped"]
    public_pass = scoring["functional"]["public_pass"]
    hidden_pass = scoring["functional"]["hidden_pass"]

    print(f"Run completed: {run_id}")
    print(f"Run directory: {run_dir}")
    print(f"Score: {total} (core={scoring['totals']['core']}, bonus={scoring['totals']['bonus']}, penalties={scoring['totals']['penalties']})")
    print(f"Public tests: {'PASS' if public_pass else 'FAIL'}")
    print(f"Hidden tests: {'PASS' if hidden_pass else 'FAIL'}")
    print(f"Report: {report_path}")
    if invalid_reasons:
        print(f"Run validity: INVALID ({', '.join(invalid_reasons)})")
    else:
        print("Run validity: valid")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
