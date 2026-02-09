#!/usr/bin/env python3

import argparse
import datetime as dt
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def run_cmd(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
    )


def safe_mean(values: list[float]) -> float | None:
    if not values:
        return None
    return sum(values) / len(values)


def safe_median(values: list[float]) -> float | None:
    if not values:
        return None
    return statistics.median(values)


def safe_min(values: list[float]) -> float | None:
    if not values:
        return None
    return min(values)


def safe_max(values: list[float]) -> float | None:
    if not values:
        return None
    return max(values)


def aggregate_counts(values: list[str]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for value in values:
        counts[value] = counts.get(value, 0) + 1
    return dict(sorted(counts.items(), key=lambda item: item[0]))


def numeric_reports(reports: list[dict[str, Any]], path: list[str]) -> list[float]:
    values: list[float] = []
    for report in reports:
        cur: Any = report
        ok = True
        for key in path:
            if not isinstance(cur, dict) or key not in cur:
                ok = False
                break
            cur = cur[key]
        if ok and isinstance(cur, (int, float)):
            values.append(float(cur))
    return values


def bool_reports(reports: list[dict[str, Any]], path: list[str]) -> list[bool]:
    values: list[bool] = []
    for report in reports:
        cur: Any = report
        ok = True
        for key in path:
            if not isinstance(cur, dict) or key not in cur:
                ok = False
                break
            cur = cur[key]
        if ok and isinstance(cur, bool):
            values.append(cur)
    return values


def load_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        if isinstance(value, dict):
            return value
    except json.JSONDecodeError:
        return None
    return None


def build_summary_markdown(summary: dict[str, Any]) -> str:
    runs = summary["runs"]
    aggregates = summary["aggregates"]

    lines: list[str] = []
    lines.append(f"# takbench batch summary: {summary['batch_id']}")
    lines.append("")
    lines.append(f"- Created: {summary['created_at']}")
    lines.append(f"- Objective: {summary['objective']}")
    lines.append(f"- Planned runs: {summary['planned_runs']}")
    lines.append(f"- Completed harness invocations: {aggregates['invocation_count']}")
    lines.append(f"- Reports loaded: {aggregates['report_count']}")
    lines.append("")

    lines.append("## Outcome rates")
    lines.append("")
    lines.append(f"- Valid runs: {aggregates['valid_runs']} / {aggregates['report_count']}")
    lines.append(f"- Public pass rate: {aggregates['public_pass_rate']:.2%}" if aggregates["public_pass_rate"] is not None else "- Public pass rate: n/a")
    lines.append(f"- Hidden pass rate: {aggregates['hidden_pass_rate']:.2%}" if aggregates["hidden_pass_rate"] is not None else "- Hidden pass rate: n/a")
    lines.append("")

    lines.append("## Score summary")
    lines.append("")
    score_stats = aggregates["score_stats"]
    lines.append(f"- Total score mean: {score_stats['total_mean']:.2f}" if score_stats["total_mean"] is not None else "- Total score mean: n/a")
    lines.append(f"- Total score median: {score_stats['total_median']:.2f}" if score_stats["total_median"] is not None else "- Total score median: n/a")
    lines.append(f"- Total score min/max: {score_stats['total_min']:.2f} / {score_stats['total_max']:.2f}" if score_stats["total_min"] is not None else "- Total score min/max: n/a")
    lines.append("")

    lines.append("## Command activity (mean)")
    lines.append("")
    command_stats = aggregates["command_activity_stats"]
    lines.append(
        f"- tak commands: {command_stats['tak_command_mean']:.2f}"
        if command_stats["tak_command_mean"] is not None
        else "- tak commands: n/a"
    )
    lines.append(
        f"- tak verify commands: {command_stats['tak_verify_command_mean']:.2f}"
        if command_stats["tak_verify_command_mean"] is not None
        else "- tak verify commands: n/a"
    )
    lines.append(
        f"- pytest commands: {command_stats['pytest_command_mean']:.2f}"
        if command_stats["pytest_command_mean"] is not None
        else "- pytest commands: n/a"
    )
    lines.append(
        f"- extracted command lines: {command_stats['extracted_command_mean']:.2f}"
        if command_stats["extracted_command_mean"] is not None
        else "- extracted command lines: n/a"
    )
    lines.append("")

    lines.append("## Invalid reason counts")
    lines.append("")
    invalid_reasons = aggregates["invalid_reason_counts"]
    if invalid_reasons:
        for reason, count in invalid_reasons.items():
            lines.append(f"- `{reason}`: {count}")
    else:
        lines.append("- none")
    lines.append("")

    lines.append("## Per-run summary")
    lines.append("")
    lines.append("| run_id | harness_exit | report_loaded | valid | total_score | public | hidden |")
    lines.append("|---|---:|---:|---:|---:|---:|---:|")
    for run in runs:
        public_pass = run.get("public_pass")
        hidden_pass = run.get("hidden_pass")
        lines.append(
            "| {run_id} | {exit_code} | {loaded} | {valid} | {score} | {public} | {hidden} |".format(
                run_id=run["run_id"],
                exit_code=run["harness_exit_code"],
                loaded="yes" if run["report_loaded"] else "no",
                valid="yes" if run.get("valid") else "no",
                score=f"{run.get('total_score'):.2f}" if isinstance(run.get("total_score"), (int, float)) else "n/a",
                public="pass" if public_pass else ("fail" if public_pass is False else "n/a"),
                hidden="pass" if hidden_pass else ("fail" if hidden_pass is False else "n/a"),
            )
        )

    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="Run takbench repeatedly and aggregate diagnostics")
    parser.add_argument("--objective", type=Path, default=Path("bench/objectives/markdown_parser_v1"))
    parser.add_argument("--worker-cmd", required=True)
    parser.add_argument("--hidden-test-cmd", type=str, default="")
    parser.add_argument("--count", type=int, default=5)
    parser.add_argument("--runs-dir", type=Path, default=Path("bench/runs"))
    parser.add_argument("--batch-id", type=str, default="")
    parser.add_argument("--time-budget-minutes", type=int, default=0)
    parser.add_argument("--session-prefix", type=str, default="takbench")
    parser.add_argument("--sleep-between-seconds", type=int, default=0)
    parser.add_argument("--fail-fast", action="store_true")
    parser.add_argument("--skip-tak-init", action="store_true")
    parser.add_argument("--allow-missing-hidden-tests", action="store_true")
    parser.add_argument("--allow-non-uv", action="store_true")

    args = parser.parse_args()

    if args.count < 1:
        print("Error: --count must be >= 1", file=sys.stderr)
        return 2

    objective_path = args.objective.resolve()
    runs_dir = args.runs_dir.resolve()

    root = Path(__file__).resolve().parent
    harness_path = root / "takbench.py"
    if not harness_path.exists():
        print(f"Error: harness script not found: {harness_path}", file=sys.stderr)
        return 2

    batch_id = args.batch_id.strip()
    if not batch_id:
        batch_id = dt.datetime.now().strftime("batch_%Y%m%d_%H%M%S")

    batch_dir = (runs_dir / "batches" / batch_id).resolve()
    logs_dir = batch_dir / "logs"
    batch_dir.mkdir(parents=True, exist_ok=False)
    logs_dir.mkdir(parents=True, exist_ok=False)

    run_summaries: list[dict[str, Any]] = []

    for i in range(args.count):
        run_id = f"{batch_id}_{i + 1:03d}"
        cmd = [
            sys.executable,
            str(harness_path),
            "--objective",
            str(objective_path),
            "--worker-cmd",
            args.worker_cmd,
            "--runs-dir",
            str(runs_dir),
            "--run-id",
            run_id,
            "--session-prefix",
            args.session_prefix,
        ]

        if args.hidden_test_cmd.strip():
            cmd.extend(["--hidden-test-cmd", args.hidden_test_cmd.strip()])

        if args.time_budget_minutes > 0:
            cmd.extend(["--time-budget-minutes", str(args.time_budget_minutes)])

        if args.skip_tak_init:
            cmd.append("--skip-tak-init")
        if args.allow_missing_hidden_tests:
            cmd.append("--allow-missing-hidden-tests")
        if args.allow_non_uv:
            cmd.append("--allow-non-uv")

        started_at = utc_now()
        result = run_cmd(cmd)
        ended_at = utc_now()

        (logs_dir / f"{run_id}.stdout.log").write_text(result.stdout, encoding="utf-8")
        (logs_dir / f"{run_id}.stderr.log").write_text(result.stderr, encoding="utf-8")

        report_path = (runs_dir / run_id / "report.json").resolve()
        report = load_json(report_path)

        run_summary: dict[str, Any] = {
            "run_id": run_id,
            "started_at": started_at,
            "ended_at": ended_at,
            "harness_command": cmd,
            "harness_exit_code": result.returncode,
            "report_path": str(report_path),
            "report_loaded": report is not None,
            "stdout_log": str((logs_dir / f"{run_id}.stdout.log").resolve()),
            "stderr_log": str((logs_dir / f"{run_id}.stderr.log").resolve()),
        }

        if report is not None:
            run_summary.update(
                {
                    "valid": report.get("valid"),
                    "invalid_reasons": report.get("invalid_reasons", []),
                    "total_score": report.get("scores", {}).get("totals", {}).get("clamped"),
                    "public_pass": report.get("scores", {}).get("functional", {}).get("public_pass"),
                    "hidden_pass": report.get("scores", {}).get("functional", {}).get("hidden_pass"),
                    "change_pass": report.get("scores", {}).get("functional", {}).get("change_probe_pass"),
                    "task_count": report.get("metrics", {}).get("tak", {}).get("task_count"),
                    "commit_count": report.get("metrics", {}).get("git", {}).get("commit_count"),
                }
            )

        run_summaries.append(run_summary)

        print(
            f"[{i + 1}/{args.count}] run_id={run_id} exit={result.returncode} "
            f"report={'yes' if report is not None else 'no'}"
        )

        if args.fail_fast and result.returncode != 0:
            print("Fail-fast enabled: stopping batch.")
            break

        if args.sleep_between_seconds > 0 and i < args.count - 1:
            time.sleep(args.sleep_between_seconds)

    loaded_reports = []
    for run in run_summaries:
        if run.get("report_loaded"):
            report_path = Path(str(run["report_path"]))
            report = load_json(report_path)
            if report is not None:
                loaded_reports.append(report)

    valid_values = bool_reports(loaded_reports, ["valid"])
    public_values = bool_reports(loaded_reports, ["scores", "functional", "public_pass"])
    hidden_values = bool_reports(loaded_reports, ["scores", "functional", "hidden_pass"])

    total_scores = numeric_reports(loaded_reports, ["scores", "totals", "clamped"])
    core_scores = numeric_reports(loaded_reports, ["scores", "totals", "core"])
    penalty_scores = numeric_reports(loaded_reports, ["scores", "totals", "penalties"])
    tak_scores = numeric_reports(loaded_reports, ["scores", "tak_workflow", "score"])
    git_scores = numeric_reports(loaded_reports, ["scores", "git_discipline", "score"])

    task_counts = numeric_reports(loaded_reports, ["metrics", "tak", "task_count"])
    commit_counts = numeric_reports(loaded_reports, ["metrics", "git", "commit_count"])

    tak_command_counts = numeric_reports(loaded_reports, ["metrics", "transcript", "tak_mentions"])
    tak_verify_command_counts = numeric_reports(
        loaded_reports,
        ["metrics", "transcript", "tak_verify_mentions"],
    )
    pytest_command_counts = numeric_reports(loaded_reports, ["metrics", "transcript", "pytest_mentions"])
    extracted_command_counts = numeric_reports(
        loaded_reports,
        ["metrics", "transcript", "extracted_command_count"],
    )

    invalid_reason_values: list[str] = []
    end_reasons: list[str] = []
    for report in loaded_reports:
        invalids = report.get("invalid_reasons", [])
        if isinstance(invalids, list):
            for item in invalids:
                invalid_reason_values.append(str(item))

        session = report.get("timestamps", {}).get("session", {})
        if isinstance(session, dict):
            end_reason = session.get("end_reason")
            if isinstance(end_reason, str) and end_reason:
                end_reasons.append(end_reason)

    aggregates = {
        "invocation_count": len(run_summaries),
        "report_count": len(loaded_reports),
        "valid_runs": sum(1 for v in valid_values if v),
        "invalid_runs": sum(1 for v in valid_values if not v),
        "public_pass_rate": (sum(1 for v in public_values if v) / len(public_values)) if public_values else None,
        "hidden_pass_rate": (sum(1 for v in hidden_values if v) / len(hidden_values)) if hidden_values else None,
        "score_stats": {
            "total_mean": safe_mean(total_scores),
            "total_median": safe_median(total_scores),
            "total_min": safe_min(total_scores),
            "total_max": safe_max(total_scores),
            "core_mean": safe_mean(core_scores),
            "penalties_mean": safe_mean(penalty_scores),
            "tak_workflow_mean": safe_mean(tak_scores),
            "git_discipline_mean": safe_mean(git_scores),
        },
        "work_stats": {
            "task_count_mean": safe_mean(task_counts),
            "commit_count_mean": safe_mean(commit_counts),
            "task_count_median": safe_median(task_counts),
            "commit_count_median": safe_median(commit_counts),
        },
        "command_activity_stats": {
            "tak_command_mean": safe_mean(tak_command_counts),
            "tak_verify_command_mean": safe_mean(tak_verify_command_counts),
            "pytest_command_mean": safe_mean(pytest_command_counts),
            "extracted_command_mean": safe_mean(extracted_command_counts),
        },
        "invalid_reason_counts": aggregate_counts(invalid_reason_values),
        "session_end_reason_counts": aggregate_counts(end_reasons),
    }

    summary = {
        "batch_id": batch_id,
        "created_at": utc_now(),
        "objective": str(objective_path),
        "planned_runs": args.count,
        "batch_dir": str(batch_dir),
        "runs_dir": str(runs_dir),
        "runs": run_summaries,
        "aggregates": aggregates,
    }

    summary_json_path = batch_dir / "summary.json"
    summary_md_path = batch_dir / "summary.md"
    runs_jsonl_path = batch_dir / "runs.jsonl"

    summary_json_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    summary_md_path.write_text(build_summary_markdown(summary), encoding="utf-8")

    with runs_jsonl_path.open("w", encoding="utf-8") as f:
        for run in run_summaries:
            f.write(json.dumps(run, ensure_ascii=False) + "\n")

    print("Batch completed")
    print(f"batch_id: {batch_id}")
    print(f"batch_dir: {batch_dir}")
    print(f"reports: {aggregates['report_count']} / invocations: {aggregates['invocation_count']}")
    if aggregates["score_stats"]["total_mean"] is not None:
        print(
            f"total_score_mean: {aggregates['score_stats']['total_mean']:.2f} "
            f"(median={aggregates['score_stats']['total_median']:.2f})"
        )
    print(f"summary_json: {summary_json_path}")
    print(f"summary_md: {summary_md_path}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
