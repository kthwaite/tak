import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative } from "node:path";

import {
	DEFAULT_MAX_BYTES,
	DEFAULT_MAX_LINES,
	DynamicBorder,
	formatSize,
	isToolCallEventType,
	truncateHead,
	type ExtensionAPI,
	type ExtensionContext,
} from "@mariozechner/pi-coding-agent";
import { Container, type SelectItem, SelectList, Text } from "@mariozechner/pi-tui";
import { Type } from "@sinclair/typebox";

type TaskSource = "ready" | "all" | "blocked" | "in_progress" | "mine" | "blackboard" | "inbox";

type Priority = "critical" | "high" | "medium" | "low";
type TaskStatus = "pending" | "in_progress" | "done" | "cancelled";
type VerifyMode = "isolated" | "local";
type WorkAction = "start" | "stop" | "status";
type TherapistAction = "offline" | "online" | "log";

type CommandMode = "pick" | "claim" | "mesh" | "show" | "help" | "work" | "therapist";

interface TakTask {
	id: number;
	title: string;
	status: TaskStatus;
	kind: string;
	assignee?: string;
	tags?: string[];
	planning?: {
		priority?: Priority;
	};
	created_at?: string;
	updated_at?: string;
}

interface BlackboardNote {
	id: number;
	author: string;
	message: string;
	status: "open" | "closed";
	task_ids?: number[];
	tags?: string[];
	updated_at?: string;
}

interface MeshAgent {
	name: string;
	session_id: string;
	status: string;
	cwd: string;
}

interface MeshMessage {
	id: string;
	from: string;
	to: string;
	text: string;
	timestamp: string;
}

interface MeshReservation {
	agent: string;
	paths: string[];
	reason?: string;
	since: string;
}

interface TherapistObservation {
	id: string;
	timestamp: string;
	mode: TherapistAction;
	summary: string;
	session?: string;
	requested_by?: string;
	findings?: string[];
	recommendations?: string[];
	interview?: string;
	metrics?: Record<string, unknown>;
}

interface TakFilters {
	source: TaskSource;
	tag?: string;
	kind?: string;
	priority?: Priority;
	status?: TaskStatus;
	assignee?: string;
	limit?: number;
	taskId?: number;
	ackInbox?: boolean;
	verifyMode?: VerifyMode;
}

interface ParsedTakCommand {
	mode: CommandMode;
	filters: TakFilters;
	taskId?: number;
	workAction?: WorkAction;
	therapistAction?: TherapistAction;
	therapistSession?: string;
	therapistBy?: string;
}

interface WorkLoopState {
	active: boolean;
	tag?: string;
	remaining?: number;
	verifyMode: VerifyMode;
	strictReservations: boolean;
	currentTaskId?: number;
	processed: number;
}

interface TakStatusSnapshot {
	readyTasks: TakTask[];
	blockedTasks: TakTask[];
	inProgressTasks: TakTask[];
	openNotes: BlackboardNote[];
	inboxCount: number;
	peerCount: number;
	currentTask?: TakTask;
}

interface TakExecResult {
	ok: boolean;
	code: number;
	stdout: string;
	stderr: string;
	parsed?: unknown;
	errorMessage?: string;
	args: string[];
}

const SYSTEM_APPEND = `
You are operating in a repository that uses tak as the canonical task manager.

Task and coordination protocol:
1. Use tak actively for planning and execution: list, show, claim/start, handoff, finish, cancel, and context/log updates.
2. Prefer the tak_cli tool for task/mesh/blackboard operations instead of ad-hoc shell commands.
3. Prioritise work by urgency first (critical/high/medium/low), then by age (oldest first within the same priority).
4. Coordinate with peers through mesh and blackboard:
   - check mesh presence/inbox when coordinating work,
   - avoid taking work already owned by another agent,
   - reserve files before major edits when possible,
   - use blackboard notes for blockers/handoffs/heads-up communication.
5. If other agents are active on mesh, avoid stepping on their toes: communicate first, then proceed.
`;

const TAK_HELP = `
/tak [source] [filters]            Pick work from tak (default source: ready)
/tak claim [tag:<tag>]             Atomically claim next task
/tak work [tag:<tag>] [limit:<n>] [verify:isolated|local]
                                   Start/resume autonomous work loop
/tak work status|stop              Inspect or stop work loop
/tak mesh                          Insert a mesh + blackboard summary in the editor
/tak therapist [offline|online|log]
                                   Run workflow diagnosis/interview and append observations
/tak <task-id>                     Open a specific task

Sources:
- ready (default): pending + unblocked + unassigned
- blocked
- all
- in_progress
- mine
- blackboard (open notes)
- inbox (mesh inbox for current agent)

Filters (space-separated):
- tag:<tag>
- kind:<epic|feature|task|bug>
- priority:<critical|high|medium|low>
- status:<pending|in_progress|done|cancelled>
- assignee:<name>
- limit:<n>
- task:<id>         (for blackboard source)
- ack               (for inbox source)
- verify:<mode>     (for /tak work; mode = isolated | local)
- session:<id|path> (for /tak therapist online)
- by:<name>         (for /tak therapist offline|online)

Work mode notes:
- Automatically claims the next available task for you.
- When the current task is finished/handed off/cancelled, the next task is auto-claimed.
- In work mode, edits are blocked unless the path is reserved by your agent.
- With verify:isolated (default), local build/test/check commands are blocked when peers hold reservations.
`;

const COMPLETIONS = [
	"ready",
	"all",
	"blocked",
	"in_progress",
	"mine",
	"blackboard",
	"inbox",
	"pick",
	"claim",
	"work",
	"work status",
	"work stop",
	"mesh",
	"therapist",
	"therapist offline",
	"therapist online",
	"therapist log",
	"help",
	"tag:",
	"kind:task",
	"kind:bug",
	"kind:feature",
	"kind:epic",
	"priority:critical",
	"priority:high",
	"priority:medium",
	"priority:low",
	"status:pending",
	"status:in_progress",
	"status:done",
	"status:cancelled",
	"assignee:",
	"limit:20",
	"task:",
	"session:",
	"by:",
	"ack",
	"verify:isolated",
	"verify:local",
] as const;

const SOURCE_SET: Set<string> = new Set([
	"ready",
	"all",
	"blocked",
	"in_progress",
	"mine",
	"blackboard",
	"inbox",
]);

function parseTakError(stderr: string): string | undefined {
	const trimmed = stderr.trim();
	if (!trimmed) return undefined;
	try {
		const parsed = JSON.parse(trimmed) as { message?: string };
		if (parsed?.message) return parsed.message;
	} catch {
		// Ignore parse errors; fall back to raw stderr.
	}
	return trimmed;
}

function parseSource(token?: string): TaskSource | undefined {
	if (!token) return undefined;
	const normalized = token.toLowerCase();
	if (SOURCE_SET.has(normalized)) {
		return normalized as TaskSource;
	}
	return undefined;
}

function parseTakCommandInput(rawArgs: string): ParsedTakCommand {
	const tokens = rawArgs
		.trim()
		.split(/\s+/)
		.map((t) => t.trim())
		.filter(Boolean);

	const filters: TakFilters = {
		source: "ready",
	};

	if (tokens.length === 0) {
		return { mode: "pick", filters };
	}

	const first = tokens[0]!.toLowerCase();

	if (/^\d+$/.test(first)) {
		return {
			mode: "show",
			filters,
			taskId: Number.parseInt(first, 10),
		};
	}

	if (first === "help") {
		return { mode: "help", filters };
	}

	if (first === "mesh") {
		return { mode: "mesh", filters };
	}

	if (first === "therapist") {
		let therapistAction: TherapistAction = "offline";
		let tokenStart = 1;
		const second = tokens[1]?.toLowerCase();
		if (second === "offline" || second === "online" || second === "log") {
			therapistAction = second;
			tokenStart = 2;
		}

		let therapistSession: string | undefined;
		let therapistBy: string | undefined;

		for (const token of tokens.slice(tokenStart)) {
			const [rawKey, ...rawValueParts] = token.split(":");
			if (rawKey && rawValueParts.length > 0) {
				const key = rawKey.toLowerCase();
				const value = rawValueParts.join(":").trim();
				if (key === "session" && value) {
					therapistSession = value;
					continue;
				}
				if (key === "by" && value) {
					therapistBy = value;
					continue;
				}
			}
			applyFilterToken(token, filters);
		}

		return {
			mode: "therapist",
			filters,
			therapistAction,
			therapistSession,
			therapistBy,
		};
	}

	if (first === "claim") {
		for (const token of tokens.slice(1)) applyFilterToken(token, filters);
		return { mode: "claim", filters };
	}

	if (first === "work") {
		let workAction: WorkAction = "start";
		let filterStart = 1;
		const second = tokens[1]?.toLowerCase();
		if (second === "stop" || second === "status" || second === "start") {
			workAction = second;
			filterStart = 2;
		}
		for (const token of tokens.slice(filterStart)) applyFilterToken(token, filters);
		return { mode: "work", filters, workAction };
	}

	const directSource = parseSource(first);
	const remaining = directSource ? tokens.slice(1) : first === "pick" ? tokens.slice(1) : tokens;
	if (directSource) {
		filters.source = directSource;
	}

	for (const token of remaining) {
		applyFilterToken(token, filters);
	}

	return { mode: "pick", filters };
}

function applyFilterToken(token: string, filters: TakFilters): void {
	const normalized = token.toLowerCase();
	const source = parseSource(normalized);
	if (source) {
		filters.source = source;
		return;
	}

	if (normalized === "ack") {
		filters.ackInbox = true;
		return;
	}

	const [rawKey, ...rawValueParts] = token.split(":");
	if (!rawKey || rawValueParts.length === 0) return;

	const key = rawKey.toLowerCase();
	const value = rawValueParts.join(":").trim();
	if (!value) return;

	switch (key) {
		case "tag":
			filters.tag = value;
			break;
		case "kind":
			filters.kind = value;
			break;
		case "priority":
			if (["critical", "high", "medium", "low"].includes(value.toLowerCase())) {
				filters.priority = value.toLowerCase() as Priority;
			}
			break;
		case "status":
			if (["pending", "in_progress", "done", "cancelled"].includes(value.toLowerCase())) {
				filters.status = value.toLowerCase() as TaskStatus;
			}
			break;
		case "assignee":
			filters.assignee = value;
			break;
		case "limit": {
			const parsed = Number.parseInt(value, 10);
			if (Number.isFinite(parsed) && parsed > 0) filters.limit = parsed;
			break;
		}
		case "task": {
			const parsed = Number.parseInt(value, 10);
			if (Number.isFinite(parsed) && parsed > 0) filters.taskId = parsed;
			break;
		}
		case "verify":
			if (value.toLowerCase() === "isolated" || value.toLowerCase() === "local") {
				filters.verifyMode = value.toLowerCase() as VerifyMode;
			}
			break;
	}
}

function priorityRank(task: TakTask): number {
	switch (task.planning?.priority) {
		case "critical":
			return 0;
		case "high":
			return 1;
		case "medium":
			return 2;
		case "low":
			return 3;
		default:
			return 4;
	}
}

function parseTimestamp(value?: string): number {
	if (!value) return Number.MAX_SAFE_INTEGER;
	const ts = Date.parse(value);
	return Number.isFinite(ts) ? ts : Number.MAX_SAFE_INTEGER;
}

function sortTasksUrgentThenOldest(tasks: TakTask[]): TakTask[] {
	return [...tasks].sort((a, b) => {
		const priorityDiff = priorityRank(a) - priorityRank(b);
		if (priorityDiff !== 0) return priorityDiff;

		const createdDiff = parseTimestamp(a.created_at) - parseTimestamp(b.created_at);
		if (createdDiff !== 0) return createdDiff;

		return a.id - b.id;
	});
}

function taskAgeLabel(task: TakTask): string {
	const ts = parseTimestamp(task.created_at);
	if (!Number.isFinite(ts) || ts === Number.MAX_SAFE_INTEGER) return "age:?";
	const days = Math.floor((Date.now() - ts) / (1000 * 60 * 60 * 24));
	if (days <= 0) return "age:<1d";
	return `age:${days}d`;
}

async function runTak(
	pi: ExtensionAPI,
	args: string[],
	options?: { json?: boolean; timeoutMs?: number; signal?: AbortSignal },
): Promise<TakExecResult> {
	const json = options?.json ?? true;
	const finalArgs = [...args];

	if (json && !finalArgs.includes("--format") && !finalArgs.includes("--pretty")) {
		finalArgs.push("--format", "json");
	}

	const result = await pi.exec("tak", finalArgs, {
		timeout: options?.timeoutMs ?? 15000,
		signal: options?.signal,
	});

	const stdout = (result.stdout ?? "").trim();
	const stderr = (result.stderr ?? "").trim();

	let parsed: unknown;
	if (json && stdout) {
		try {
			parsed = JSON.parse(stdout);
		} catch {
			// Keep raw stdout when parsing fails.
		}
	}

	const ok = result.code === 0;
	let errorMessage: string | undefined;
	if (!ok) {
		errorMessage = parseTakError(stderr) ?? (stdout || `tak exited with code ${result.code}`);
	}

	return {
		ok,
		code: result.code,
		stdout,
		stderr,
		parsed,
		errorMessage,
		args: finalArgs,
	};
}

function buildTaskListArgs(filters: TakFilters, agentName?: string): string[] {
	const args = ["list"];

	switch (filters.source) {
		case "ready":
			args.push("--available");
			break;
		case "blocked":
			args.push("--blocked");
			break;
		case "in_progress":
			args.push("--status", "in_progress");
			break;
		case "mine":
			if (filters.assignee ?? agentName) {
				args.push("--assignee", filters.assignee ?? agentName!);
			}
			break;
		case "all":
		default:
			break;
	}

	if (filters.status && filters.source !== "in_progress") args.push("--status", filters.status);
	if (filters.kind) args.push("--kind", filters.kind);
	if (filters.tag) args.push("--tag", filters.tag);
	if (filters.priority) args.push("--priority", filters.priority);
	if (filters.assignee && filters.source !== "mine") args.push("--assignee", filters.assignee);

	return args;
}

function normalizePathLikeTak(path: string): string {
	const cleaned = path.replace(/\\/g, "/").replace(/^@/, "");
	const parts: string[] = [];
	for (const part of cleaned.split("/")) {
		if (!part || part === ".") continue;
		if (part === "..") {
			parts.pop();
			continue;
		}
		parts.push(part);
	}
	const normalized = parts.join("/");
	if (cleaned.endsWith("/") && normalized) return `${normalized}/`;
	return normalized;
}

function pathsConflict(a: string, b: string): boolean {
	const na = normalizePathLikeTak(a);
	const nb = normalizePathLikeTak(b);
	if (na === nb) return true;

	const ta = na.replace(/\/+$/, "");
	const tb = nb.replace(/\/+$/, "");
	if (ta === tb) return true;

	return ta.startsWith(`${tb}/`) || tb.startsWith(`${ta}/`);
}

function toRepoRelativePath(cwd: string, inputPath: string): string {
	const withoutAt = inputPath.replace(/^@/, "");
	if (!withoutAt) return "";
	if (!isAbsolute(withoutAt)) return normalizePathLikeTak(withoutAt);

	const rel = relative(cwd, withoutAt).replace(/\\/g, "/");
	if (!rel.startsWith("..")) return normalizePathLikeTak(rel);
	return normalizePathLikeTak(withoutAt);
}

function loadReservations(cwd: string): MeshReservation[] {
	const reservationsPath = join(cwd, ".tak", "runtime", "mesh", "reservations.json");
	if (!existsSync(reservationsPath)) return [];
	try {
		const content = readFileSync(reservationsPath, "utf-8");
		const parsed = JSON.parse(content) as unknown;
		if (!Array.isArray(parsed)) return [];
		return parsed as MeshReservation[];
	} catch {
		return [];
	}
}

function reservationsOwnedBy(agentName: string, reservations: MeshReservation[]): MeshReservation[] {
	return reservations.filter((reservation) => reservation.agent === agentName);
}

function hasOwnedReservationForPath(agentName: string, targetPath: string, reservations: MeshReservation[]): boolean {
	return reservationsOwnedBy(agentName, reservations).some((reservation) =>
		reservation.paths.some((reservedPath) => pathsConflict(targetPath, reservedPath)),
	);
}

function hasForeignReservations(agentName: string, reservations: MeshReservation[]): boolean {
	return reservations.some((reservation) => reservation.agent !== agentName);
}

function isLikelyBuildOrTestCommand(command: string): boolean {
	const normalized = command.toLowerCase();
	return [
		/\bcargo\s+(build|check|test|clippy|run)\b/,
		/\b(npm|pnpm|yarn)\s+(test|run\s+test|run\s+lint|run\s+build|lint|build)\b/,
		/\bpytest\b/,
		/\b(go\s+test|gradle\s+test|mvn\s+test|ctest|jest|vitest)\b/,
	].some((pattern) => pattern.test(normalized));
}

function formatWorkLoopStatus(workLoop: WorkLoopState): string {
	if (!workLoop.active) return "work: inactive";
	const parts = ["work: active"];
	if (workLoop.currentTaskId) parts.push(`task=#${workLoop.currentTaskId}`);
	if (workLoop.tag) parts.push(`tag=${workLoop.tag}`);
	if (workLoop.remaining !== undefined) parts.push(`remaining=${workLoop.remaining}`);
	parts.push(`verify=${workLoop.verifyMode}`);
	parts.push(`processed=${workLoop.processed}`);
	return parts.join(" | ");
}

function truncateText(text: string, maxChars: number): string {
	const normalized = text.replace(/\s+/g, " ").trim();
	if (normalized.length <= maxChars) return normalized;
	return `${normalized.slice(0, Math.max(1, maxChars - 1))}…`;
}

function formatTherapistObservation(observation: TherapistObservation): string {
	const lines: string[] = [];
	lines.push(`# tak therapist ${observation.mode}`);
	lines.push(`id: ${observation.id}`);
	lines.push(`timestamp: ${observation.timestamp}`);
	if (observation.session) lines.push(`session: ${observation.session}`);
	if (observation.requested_by) lines.push(`requested_by: ${observation.requested_by}`);
	lines.push(`summary: ${observation.summary}`);

	if (observation.findings?.length) {
		lines.push("", "findings:");
		for (const finding of observation.findings) {
			lines.push(`- ${finding}`);
		}
	}

	if (observation.recommendations?.length) {
		lines.push("", "recommendations:");
		for (const recommendation of observation.recommendations) {
			lines.push(`- ${recommendation}`);
		}
	}

	if (observation.interview) {
		lines.push("", "interview:", observation.interview);
	}

	return lines.join("\n");
}

function formatTherapistLog(observations: TherapistObservation[]): string {
	if (observations.length === 0) {
		return "No tak therapist observations found.";
	}

	const lines: string[] = ["# tak therapist log"];
	for (const observation of observations) {
		lines.push(`- ${observation.timestamp} [${observation.mode}] ${truncateText(observation.summary, 110)}`);
	}
	return lines.join("\n");
}

function buildStatusChip(
	theme: ExtensionContext["ui"]["theme"],
	icon: string,
	label: string,
	count: number,
	activeColor: "success" | "warning" | "accent" | "muted",
): string {
	const tone = count > 0 ? activeColor : "dim";
	return theme.fg(tone, `${icon} ${label} ${count}`);
}

function formatTaskBadge(task: TakTask): string {
	const title = truncateText(task.title, 28);
	return `#${task.id} ${title}`;
}

function buildTakStatusBar(
	ctx: ExtensionContext,
	snapshot: TakStatusSnapshot,
	workLoop: WorkLoopState,
	agentName?: string,
): string {
	const theme = ctx.ui.theme;
	const parts = [
		theme.fg(agentName ? "accent" : "dim", `◉ agent ${agentName ?? "(unjoined)"}`),
		buildStatusChip(theme, "●", "ready", snapshot.readyTasks.length, "success"),
		buildStatusChip(theme, "◌", "blocked", snapshot.blockedTasks.length, "warning"),
		buildStatusChip(theme, "◐", "active", snapshot.inProgressTasks.length, "accent"),
		buildStatusChip(theme, "◎", "peers", snapshot.peerCount, "muted"),
		buildStatusChip(theme, "✉", "inbox", snapshot.inboxCount, "warning"),
		buildStatusChip(theme, "⚑", "bb", snapshot.openNotes.length, "accent"),
	];

	if (snapshot.currentTask) {
		parts.push(ctx.ui.theme.fg("accent", `▶ ${formatTaskBadge(snapshot.currentTask)}`));
	}

	if (workLoop.active) {
		const mode = workLoop.currentTaskId ? `#${workLoop.currentTaskId}` : "idle";
		parts.push(theme.fg("warning", `↻ work ${mode}`));
	}

	return parts.join(theme.fg("dim", "  "));
}

async function pickFromList(
	ctx: ExtensionContext,
	title: string,
	items: SelectItem[],
	footerHint?: string,
): Promise<string | null> {
	if (items.length === 0) return null;

	return ctx.ui.custom<string | null>((tui, theme, _kb, done) => {
		const container = new Container();
		container.addChild(new DynamicBorder((s: string) => theme.fg("accent", s)));
		container.addChild(new Text(theme.fg("accent", theme.bold(title))));

		const list = new SelectList(items, Math.min(Math.max(items.length, 4), 16), {
			selectedPrefix: (t) => theme.fg("accent", t),
			selectedText: (t) => theme.fg("accent", t),
			description: (t) => theme.fg("muted", t),
			scrollInfo: (t) => theme.fg("dim", t),
			noMatch: (t) => theme.fg("warning", t),
		});

		list.onSelect = (item) => done(String(item.value));
		list.onCancel = () => done(null);
		container.addChild(list);

		container.addChild(new Text(theme.fg("dim", footerHint ?? "↑↓ navigate • type to filter • enter select • esc cancel")));
		container.addChild(new DynamicBorder((s: string) => theme.fg("accent", s)));

		return {
			render: (w: number) => container.render(w),
			invalidate: () => container.invalidate(),
			handleInput(data: string) {
				list.handleInput(data);
				tui.requestRender();
			},
		};
	});
}

function buildTaskEditorText(
	task: TakTask,
	agentName?: string,
	openNotes?: BlackboardNote[],
	options?: { workLoop?: WorkLoopState },
): string {
	const priority = task.planning?.priority ?? "unprioritized";
	const assignee = task.assignee ?? "unassigned";
	const tags = task.tags?.length ? task.tags.join(", ") : "-";
	const linkedNotes = (openNotes ?? []).map((n) => `- [B${n.id}] ${n.message}`).join("\n");
	const workLoop = options?.workLoop;

	const lines = [
		`Selected tak task #${task.id}: ${task.title}`,
		`status: ${task.status} | priority: ${priority} | assignee: ${assignee}`,
		`tags: ${tags}`,
		"",
		"Suggested next steps:",
		`1. tak show ${task.id}`,
		agentName ? `2. tak start ${task.id} --assignee ${agentName}` : `2. tak start ${task.id} --assignee <agent-name>`,
		agentName
			? `3. Reserve touched paths before major edits: tak mesh reserve --name ${agentName} --path <path> --reason task-${task.id}`
			: `3. Reserve touched paths before major edits: tak mesh reserve --name <agent-name> --path <path> --reason task-${task.id}`,
		`4. Use blackboard for coordination notes: tak blackboard post --from ${agentName ?? "<agent-name>"} --message \"...\" --task ${task.id}`,
	];

	if (workLoop?.active) {
		lines.push(
			"",
			`Work loop active (verify=${workLoop.verifyMode}).`,
			"- Finish with: tak finish <task-id>",
			"- Blocked/unable: tak handoff <task-id> --summary \"...\" (or tak cancel --reason)",
			"- Next task auto-claims on the next turn once this task is no longer in progress.",
			"- Edit tools are blocked unless the path is reserved by your agent.",
		);
	}

	lines.push(
		"",
		linkedNotes ? "Open blackboard notes:\n" + linkedNotes : "No open blackboard notes linked to this task.",
	);

	return lines.join("\n");
}

function truncationNotice(text: string): string {
	const truncation = truncateHead(text, {
		maxLines: DEFAULT_MAX_LINES,
		maxBytes: DEFAULT_MAX_BYTES,
	});
	if (!truncation.truncated) return truncation.content;
	return [
		truncation.content,
		"",
		`[Output truncated: showing ${truncation.outputLines} of ${truncation.totalLines} lines (${formatSize(truncation.outputBytes)} of ${formatSize(truncation.totalBytes)}).]`,
	].join("\n");
}

export default function takPiExtension(pi: ExtensionAPI) {
	let hasTakRepo = false;
	let takAvailable = false;
	let meshJoined = false;
	let agentName: string | undefined;
	let peerCount = 0;
	const seenInboxMessageIds = new Set<string>();
	let workLoop: WorkLoopState = {
		active: false,
		verifyMode: "isolated",
		strictReservations: true,
		processed: 0,
	};

	function integrationEnabled(): boolean {
		return hasTakRepo && takAvailable;
	}

	function clearUi(ctx: ExtensionContext): void {
		ctx.ui.setStatus("tak", undefined);
		ctx.ui.setWidget("tak-ready", undefined);
	}

	function resetWorkLoop(overrides?: Partial<WorkLoopState>): void {
		workLoop = {
			active: false,
			verifyMode: "isolated",
			strictReservations: true,
			processed: 0,
			...overrides,
		};
	}

	async function releaseOwnReservations(): Promise<void> {
		if (!agentName) return;
		await runTak(pi, ["mesh", "release", "--name", agentName, "--all"]);
	}

	async function claimNextWorkTask(ctx: ExtensionContext): Promise<boolean> {
		if (!agentName) {
			ctx.ui.notify("/tak work requires mesh agent identity", "warning");
			return false;
		}

		const claimArgs = ["claim", "--assignee", agentName];
		if (workLoop.tag) {
			claimArgs.push("--tag", workLoop.tag);
		}

		const claimResult = await runTak(pi, claimArgs);
		if (!claimResult.ok || !claimResult.parsed || typeof claimResult.parsed !== "object") {
			return false;
		}

		const task = claimResult.parsed as TakTask;
		workLoop.currentTaskId = task.id;

		const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", String(task.id)]);
		const notes = Array.isArray(notesResult.parsed) ? (notesResult.parsed as BlackboardNote[]) : [];
		ctx.ui.setEditorText(buildTaskEditorText(task, agentName, notes, { workLoop }));
		ctx.ui.notify(`Work loop claimed task #${task.id}: ${task.title}`, "info");
		return true;
	}

	async function syncWorkLoop(ctx: ExtensionContext): Promise<void> {
		if (!integrationEnabled() || !workLoop.active || !agentName) return;

		if (workLoop.currentTaskId !== undefined) {
			const showResult = await runTak(pi, ["show", String(workLoop.currentTaskId)]);
			if (!showResult.ok || !showResult.parsed || typeof showResult.parsed !== "object") {
				ctx.ui.notify(
					showResult.errorMessage ?? `Could not refresh work-loop task #${workLoop.currentTaskId}`,
					"warning",
				);
				workLoop.currentTaskId = undefined;
			} else {
				const task = showResult.parsed as TakTask;
				const stillMine = task.status === "in_progress" && task.assignee === agentName;
				if (stillMine) {
					return;
				}

				workLoop.currentTaskId = undefined;
				workLoop.processed += 1;
				if (workLoop.remaining !== undefined && workLoop.remaining > 0) {
					workLoop.remaining -= 1;
				}
				await releaseOwnReservations();
			}
		}

		if (workLoop.remaining !== undefined && workLoop.remaining <= 0) {
			ctx.ui.notify("Work loop finished requested task limit.", "info");
			resetWorkLoop();
			return;
		}

		if (workLoop.currentTaskId === undefined) {
			const inProgressResult = await runTak(pi, ["list", "--status", "in_progress", "--assignee", agentName]);
			if (Array.isArray(inProgressResult.parsed) && inProgressResult.parsed.length > 0) {
				const mine = sortTasksUrgentThenOldest(inProgressResult.parsed as TakTask[]);
				const existing = mine[0]!;
				workLoop.currentTaskId = existing.id;
				const notesResult = await runTak(pi, [
					"blackboard",
					"list",
					"--status",
					"open",
					"--task",
					String(existing.id),
				]);
				const notes = Array.isArray(notesResult.parsed) ? (notesResult.parsed as BlackboardNote[]) : [];
				ctx.ui.setEditorText(buildTaskEditorText(existing, agentName, notes, { workLoop }));
				ctx.ui.notify(`Work loop attached to in-progress task #${existing.id}: ${existing.title}`, "info");
				return;
			}
		}

		const claimed = await claimNextWorkTask(ctx);
		if (!claimed) {
			ctx.ui.notify("Work loop stopped: no claimable task available.", "info");
			resetWorkLoop();
		}
	}

	async function refreshStatus(ctx: ExtensionContext): Promise<void> {
		if (!integrationEnabled()) {
			clearUi(ctx);
			return;
		}

		const [readyResult, blockedResult, inProgressResult, blackboardResult, meshListResult] = await Promise.all([
			runTak(pi, ["list", "--available"]),
			runTak(pi, ["list", "--blocked"]),
			runTak(pi, ["list", "--status", "in_progress"]),
			runTak(pi, ["blackboard", "list", "--status", "open"]),
			runTak(pi, ["mesh", "list"]),
		]);

		const readyTasks = Array.isArray(readyResult.parsed) ? sortTasksUrgentThenOldest(readyResult.parsed as TakTask[]) : [];
		const blockedTasks = Array.isArray(blockedResult.parsed)
			? sortTasksUrgentThenOldest(blockedResult.parsed as TakTask[])
			: [];
		const inProgressTasks = Array.isArray(inProgressResult.parsed)
			? sortTasksUrgentThenOldest(inProgressResult.parsed as TakTask[])
			: [];
		const openNotes = Array.isArray(blackboardResult.parsed) ? (blackboardResult.parsed as BlackboardNote[]) : [];

		if (Array.isArray(meshListResult.parsed)) {
			const agents = meshListResult.parsed as MeshAgent[];
			peerCount = agents.filter((a) => a.name !== agentName).length;
		}

		let inboxCount = 0;
		if (agentName) {
			const inboxResult = await runTak(pi, ["mesh", "inbox", "--name", agentName]);
			if (Array.isArray(inboxResult.parsed)) {
				const messages = inboxResult.parsed as MeshMessage[];
				inboxCount = messages.length;

				const unseen = messages.filter((m) => !seenInboxMessageIds.has(m.id));
				for (const message of messages) seenInboxMessageIds.add(message.id);

				if (unseen.length > 0 && seenInboxMessageIds.size > unseen.length) {
					ctx.ui.notify(`${unseen.length} new tak mesh message(s). Use /tak inbox.`, "info");
				}
			}
		}

		const currentTask = agentName
			? inProgressTasks.find((task) => task.assignee === agentName)
			: undefined;

		const snapshot: TakStatusSnapshot = {
			readyTasks,
			blockedTasks,
			inProgressTasks,
			openNotes,
			inboxCount,
			peerCount,
			currentTask,
		};

		ctx.ui.setStatus("tak", buildTakStatusBar(ctx, snapshot, workLoop, agentName));

		if (
			snapshot.readyTasks.length > 0 ||
			snapshot.blockedTasks.length > 0 ||
			snapshot.currentTask ||
			snapshot.inboxCount > 0 ||
			workLoop.active
		) {
			const lines: string[] = [
				`tak board: ready ${snapshot.readyTasks.length} • blocked ${snapshot.blockedTasks.length} • active ${snapshot.inProgressTasks.length} • inbox ${snapshot.inboxCount} • bb ${snapshot.openNotes.length}`,
			];

			if (snapshot.currentTask) {
				lines.push(
					`my task: ${formatTaskBadge(snapshot.currentTask)} • ${snapshot.currentTask.status} • ${snapshot.currentTask.planning?.priority ?? "unprioritized"}`,
				);
			}

			if (snapshot.readyTasks.length > 0) {
				lines.push(
					"",
					"ready queue (urgent → oldest):",
					...snapshot.readyTasks.slice(0, 3).map((task) => {
						const priority = task.planning?.priority ?? "-";
						return `  #${task.id} [${priority}] ${truncateText(task.title, 56)}`;
					}),
				);
			} else {
				lines.push("", "ready queue: empty");
			}

			if (workLoop.active) {
				lines.push("", formatWorkLoopStatus(workLoop));
			}

			ctx.ui.setWidget("tak-ready", lines, { placement: "belowEditor" });
		} else {
			ctx.ui.setWidget("tak-ready", undefined);
		}
	}

	pi.registerTool({
		name: "tak_cli",
		label: "Tak CLI",
		description:
			"Run tak task-management commands (tasks, mesh, blackboard). Output defaults to JSON and is truncated when large.",
		parameters: Type.Object({
			args: Type.Array(Type.String({ description: "Arguments to pass to tak, without the leading 'tak'" }), {
				minItems: 1,
			}),
		}),
		async execute(_toolCallId, params, signal) {
			const result = await runTak(pi, params.args, { signal });
			const text = result.ok
				? result.parsed !== undefined
					? JSON.stringify(result.parsed, null, 2)
					: result.stdout || "ok"
				: result.errorMessage ?? result.stderr ?? result.stdout ?? `tak exited with ${result.code}`;

			return {
				content: [{ type: "text", text: truncationNotice(text) }],
				details: {
					args: result.args,
					code: result.code,
					ok: result.ok,
				},
				isError: !result.ok,
			};
		},
	});

	pi.registerCommand("tak", {
		description: "Pick and coordinate tak work (default source: ready, sorted urgent → oldest)",
		getArgumentCompletions(prefix) {
			const filtered = COMPLETIONS.filter((item) => item.startsWith(prefix));
			return filtered.length > 0 ? filtered.map((value) => ({ value, label: value })) : null;
		},
		handler: async (args, ctx) => {
			if (!integrationEnabled()) {
				ctx.ui.notify("tak integration unavailable (missing .tak/ or tak binary)", "warning");
				return;
			}

			const parsed = parseTakCommandInput(args ?? "");

			if (parsed.mode === "help") {
				ctx.ui.setEditorText(TAK_HELP.trim());
				ctx.ui.notify("Inserted /tak help into editor", "info");
				return;
			}

			if (parsed.mode === "work") {
				if (!agentName) {
					ctx.ui.notify("/tak work requires mesh agent identity", "warning");
					return;
				}

				const action = parsed.workAction ?? "start";
				if (action === "status") {
					ctx.ui.setEditorText(formatWorkLoopStatus(workLoop));
					ctx.ui.notify("Inserted /tak work status", "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "stop") {
					const hadBeenActive = workLoop.active;
					resetWorkLoop();
					await releaseOwnReservations();
					ctx.ui.notify(hadBeenActive ? "Stopped /tak work loop" : "Work loop already inactive", "info");
					await refreshStatus(ctx);
					return;
				}

				resetWorkLoop({
					active: true,
					tag: parsed.filters.tag,
					remaining: parsed.filters.limit,
					verifyMode: parsed.filters.verifyMode ?? "isolated",
					strictReservations: true,
					processed: 0,
				});
				ctx.ui.notify(`Started /tak work loop (${formatWorkLoopStatus(workLoop)})`, "info");
				await syncWorkLoop(ctx);
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "therapist") {
				const action = parsed.therapistAction ?? "offline";
				const therapistArgs = ["therapist", action];

				if ((action === "offline" || action === "log") && parsed.filters.limit) {
					therapistArgs.push("--limit", String(parsed.filters.limit));
				}

				if (action === "online" && parsed.therapistSession) {
					therapistArgs.push("--session", parsed.therapistSession);
				}

				const requestedBy = parsed.therapistBy ?? agentName;
				if ((action === "offline" || action === "online") && requestedBy) {
					therapistArgs.push("--by", requestedBy);
				}

				const therapistResult = await runTak(pi, therapistArgs, {
					timeoutMs: action === "online" ? 120000 : 30000,
				});
				if (!therapistResult.ok) {
					ctx.ui.notify(therapistResult.errorMessage ?? "tak therapist failed", "error");
					return;
				}

				if (action === "log" && Array.isArray(therapistResult.parsed)) {
					ctx.ui.setEditorText(formatTherapistLog(therapistResult.parsed as TherapistObservation[]));
				} else if (therapistResult.parsed && typeof therapistResult.parsed === "object") {
					ctx.ui.setEditorText(formatTherapistObservation(therapistResult.parsed as TherapistObservation));
				} else {
					ctx.ui.setEditorText(therapistResult.stdout || "tak therapist completed");
				}

				ctx.ui.notify(`tak therapist ${action} completed`, "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "show") {
				if (!parsed.taskId) {
					ctx.ui.notify("Task id missing", "error");
					return;
				}
				const showResult = await runTak(pi, ["show", String(parsed.taskId)]);
				if (!showResult.ok || !showResult.parsed || typeof showResult.parsed !== "object") {
					ctx.ui.notify(showResult.errorMessage ?? `Could not load task ${parsed.taskId}`, "error");
					return;
				}
				const task = showResult.parsed as TakTask;
				const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", String(task.id)]);
				const notes = Array.isArray(notesResult.parsed) ? (notesResult.parsed as BlackboardNote[]) : [];
				ctx.ui.setEditorText(buildTaskEditorText(task, agentName, notes, { workLoop }));
				ctx.ui.notify(`Loaded task #${task.id}`, "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "claim") {
				const claimArgs = ["claim"];
				if (agentName) {
					claimArgs.push("--assignee", agentName);
				}
				if (parsed.filters.tag) {
					claimArgs.push("--tag", parsed.filters.tag);
				}

				const claimResult = await runTak(pi, claimArgs);
				if (!claimResult.ok || !claimResult.parsed || typeof claimResult.parsed !== "object") {
					ctx.ui.notify(claimResult.errorMessage ?? "No task claimed", "warning");
					await refreshStatus(ctx);
					return;
				}

				const task = claimResult.parsed as TakTask;
				ctx.ui.setEditorText(buildTaskEditorText(task, agentName, undefined, { workLoop }));
				ctx.ui.notify(`Claimed task #${task.id}: ${task.title}`, "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "mesh") {
				const lines: string[] = [];
				lines.push("# tak mesh summary");
				lines.push(`agent: ${agentName ?? "(not joined)"}`);

				const agentsResult = await runTak(pi, ["mesh", "list"]);
				if (Array.isArray(agentsResult.parsed)) {
					const agents = agentsResult.parsed as MeshAgent[];
					lines.push(`agents (${agents.length}):`);
					for (const agent of agents) {
						const suffix = agent.name === agentName ? " (you)" : "";
						lines.push(`- ${agent.name}${suffix} · ${agent.status}`);
					}
				}

				if (agentName) {
					const inboxResult = await runTak(pi, ["mesh", "inbox", "--name", agentName]);
					if (Array.isArray(inboxResult.parsed)) {
						const inbox = inboxResult.parsed as MeshMessage[];
						lines.push("");
						lines.push(`inbox (${inbox.length}):`);
						for (const message of inbox.slice(-5)) {
							lines.push(`- ${message.from}: ${message.text}`);
						}
					}
				}

				const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--limit", "10"]);
				if (Array.isArray(notesResult.parsed)) {
					const notes = notesResult.parsed as BlackboardNote[];
					lines.push("");
					lines.push(`open blackboard notes (${notes.length}):`);
					for (const note of notes.slice(0, 5)) {
						lines.push(`- [B${note.id}] ${note.message}`);
					}
				}

				ctx.ui.setEditorText(lines.join("\n"));
				ctx.ui.notify("Inserted tak mesh summary", "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.filters.source === "blackboard") {
				const noteArgs = ["blackboard", "list", "--status", "open"];
				if (parsed.filters.tag) noteArgs.push("--tag", parsed.filters.tag);
				if (parsed.filters.taskId) noteArgs.push("--task", String(parsed.filters.taskId));
				if (parsed.filters.limit) noteArgs.push("--limit", String(parsed.filters.limit));

				const notesResult = await runTak(pi, noteArgs);
				if (!notesResult.ok || !Array.isArray(notesResult.parsed)) {
					ctx.ui.notify(notesResult.errorMessage ?? "Could not load blackboard notes", "error");
					return;
				}

				const notes = notesResult.parsed as BlackboardNote[];
				if (notes.length === 0) {
					ctx.ui.notify("No open blackboard notes for current filters", "info");
					await refreshStatus(ctx);
					return;
				}

				const selected = await pickFromList(
					ctx,
					"/tak blackboard",
					notes.map((note) => ({
						value: String(note.id),
						label: `[B${note.id}] ${note.message}`,
						description: `${note.author} • tasks: ${note.task_ids?.join(", ") || "-"}`,
					})),
				);

				if (!selected) return;
				const note = notes.find((n) => String(n.id) === selected);
				if (!note) return;

				const linkedTask = note.task_ids?.[0];
				if (linkedTask) {
					const showResult = await runTak(pi, ["show", String(linkedTask)]);
					if (showResult.ok && showResult.parsed && typeof showResult.parsed === "object") {
						ctx.ui.setEditorText(
							buildTaskEditorText(showResult.parsed as TakTask, agentName, [note], { workLoop }),
						);
						ctx.ui.notify(`Loaded task #${linkedTask} from blackboard note B${note.id}`, "info");
					} else {
						ctx.ui.setEditorText(`[B${note.id}] ${note.message}`);
					}
				} else {
					ctx.ui.setEditorText(`[B${note.id}] ${note.message}`);
				}

				await refreshStatus(ctx);
				return;
			}

			if (parsed.filters.source === "inbox") {
				if (!agentName) {
					ctx.ui.notify("Mesh inbox requires an agent identity", "warning");
					return;
				}

				const inboxArgs = ["mesh", "inbox", "--name", agentName];
				if (parsed.filters.ackInbox) inboxArgs.push("--ack");

				const inboxResult = await runTak(pi, inboxArgs);
				if (!inboxResult.ok || !Array.isArray(inboxResult.parsed)) {
					ctx.ui.notify(inboxResult.errorMessage ?? "Could not load inbox", "error");
					return;
				}

				const messages = inboxResult.parsed as MeshMessage[];
				if (messages.length === 0) {
					ctx.ui.notify("Mesh inbox is empty", "info");
					await refreshStatus(ctx);
					return;
				}

				const selected = await pickFromList(
					ctx,
					"/tak inbox",
					messages.map((msg) => ({
						value: msg.id,
						label: `${msg.from}: ${msg.text}`,
						description: new Date(msg.timestamp).toLocaleString(),
					})),
					parsed.filters.ackInbox
						? "Messages were acknowledged while loading"
						: "Tip: /tak inbox ack to acknowledge while reading",
				);

				if (!selected) return;
				const message = messages.find((m) => m.id === selected);
				if (!message) return;

				ctx.ui.setEditorText(`Mesh message from ${message.from}:\n\n${message.text}`);
				ctx.ui.notify(`Loaded message from ${message.from}`, "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.filters.source === "mine" && !(parsed.filters.assignee ?? agentName)) {
				ctx.ui.notify("/tak mine requires an agent identity (set TAK_AGENT or join mesh)", "warning");
				return;
			}

			const listArgs = buildTaskListArgs(parsed.filters, agentName);
			const listResult = await runTak(pi, listArgs);
			if (!listResult.ok || !Array.isArray(listResult.parsed)) {
				ctx.ui.notify(listResult.errorMessage ?? "Could not load tasks", "error");
				return;
			}

			let tasks = sortTasksUrgentThenOldest(listResult.parsed as TakTask[]);
			if (parsed.filters.limit && tasks.length > parsed.filters.limit) {
				tasks = tasks.slice(0, parsed.filters.limit);
			}

			if (tasks.length === 0) {
				ctx.ui.notify("No tasks for current source/filters", "info");
				await refreshStatus(ctx);
				return;
			}

			const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open"]);
			const notes = Array.isArray(notesResult.parsed) ? (notesResult.parsed as BlackboardNote[]) : [];
			const noteCountByTask = new Map<number, number>();
			for (const note of notes) {
				for (const taskId of note.task_ids ?? []) {
					noteCountByTask.set(taskId, (noteCountByTask.get(taskId) ?? 0) + 1);
				}
			}

			const selectedId = await pickFromList(
				ctx,
				`/tak ${parsed.filters.source} (urgent → oldest)`,
				tasks.map((task) => {
					const noteCount = noteCountByTask.get(task.id) ?? 0;
					const notePart = noteCount > 0 ? ` • bb:${noteCount}` : "";
					return {
						value: String(task.id),
						label: `#${task.id} ${task.title}`,
						description: `${task.status} • ${task.planning?.priority ?? "unprioritized"} • ${taskAgeLabel(task)}${notePart}`,
					};
				}),
			);

			if (!selectedId) return;
			const selectedTask = tasks.find((task) => String(task.id) === selectedId);
			if (!selectedTask) return;

			if (selectedTask.assignee && agentName && selectedTask.assignee !== agentName) {
				const proceed = await ctx.ui.confirm(
					"Task assigned to another agent",
					`Task #${selectedTask.id} is assigned to ${selectedTask.assignee}. Continue anyway?`,
				);
				if (!proceed) return;
			}

			const linkedNotes = notes.filter((note) => (note.task_ids ?? []).includes(selectedTask.id));
			ctx.ui.setEditorText(buildTaskEditorText(selectedTask, agentName, linkedNotes, { workLoop }));
			ctx.ui.notify(`Picked task #${selectedTask.id}`, "info");
			await refreshStatus(ctx);
		},
	});

	pi.on("session_start", async (_event, ctx) => {
		hasTakRepo = existsSync(join(ctx.cwd, ".tak"));
		takAvailable = false;
		meshJoined = false;
		agentName = undefined;
		peerCount = 0;
		seenInboxMessageIds.clear();
		resetWorkLoop();

		if (!hasTakRepo) {
			clearUi(ctx);
			return;
		}

		const version = await pi.exec("tak", ["--version"], { timeout: 5000 });
		if (version.code !== 0) {
			ctx.ui.notify("tak binary not found in PATH", "warning");
			clearUi(ctx);
			return;
		}

		takAvailable = true;

		await runTak(pi, ["reindex"], { json: false, timeoutMs: 20000 });

		const envAgent = process.env.TAK_AGENT?.trim();
		const joinArgs = ["mesh", "join"];
		if (envAgent) joinArgs.push("--name", envAgent);

		let joinResult = await runTak(pi, joinArgs);
		if (!joinResult.ok && envAgent) {
			joinResult = await runTak(pi, ["mesh", "join"]);
		}

		if (joinResult.ok && joinResult.parsed && typeof joinResult.parsed === "object") {
			agentName = (joinResult.parsed as { name?: string }).name;
			meshJoined = Boolean(agentName);
			if (agentName) ctx.ui.notify(`tak mesh joined as ${agentName}`, "info");
		}

		await refreshStatus(ctx);
	});

	pi.on("session_shutdown", async () => {
		if (!integrationEnabled() || !meshJoined || !agentName) return;
		await runTak(pi, ["mesh", "leave", "--name", agentName]);
	});

	pi.on("turn_end", async (_event, ctx) => {
		await syncWorkLoop(ctx);
		await refreshStatus(ctx);
	});

	pi.on("before_agent_start", async (event) => {
		if (!integrationEnabled()) return;
		const meshLine =
			peerCount > 0
				? `Mesh currently has ${peerCount} other active agent(s). Coordinate before overlapping work.`
				: "No other mesh agents are currently visible.";
		const workLine = workLoop.active
			? `WORK LOOP ACTIVE (${formatWorkLoopStatus(workLoop)}). Finish or handoff the current task before taking unrelated work; reserve paths before edits.`
			: "";
		return {
			systemPrompt: `${event.systemPrompt}\n\n${SYSTEM_APPEND.trim()}\n\n${meshLine}${workLine ? `\n${workLine}` : ""}`,
		};
	});

	pi.on("tool_call", async (event, ctx) => {
		if (!integrationEnabled() || !agentName) return;

		const reservations = loadReservations(ctx.cwd);

		if (isToolCallEventType("bash", event) && workLoop.active && workLoop.verifyMode === "isolated") {
			const command = event.input.command ?? "";
			if (isLikelyBuildOrTestCommand(command) && hasForeignReservations(agentName, reservations)) {
				return {
					block: true,
					reason:
						"Work loop guard: local build/test/check is blocked while peers hold reservations. Wait for reservations to clear or hand off if blocked.",
				};
			}
		}

		let pathArg: string | undefined;
		if (isToolCallEventType("write", event)) {
			pathArg = event.input.path;
		}
		if (isToolCallEventType("edit", event)) {
			pathArg = event.input.path;
		}
		if (!pathArg) return;

		const targetPath = toRepoRelativePath(ctx.cwd, pathArg);
		if (!targetPath) return;

		const conflict = reservations.find((reservation) => {
			if (reservation.agent === agentName) return false;
			return reservation.paths.some((reservedPath) => pathsConflict(targetPath, reservedPath));
		});

		if (conflict) {
			return {
				block: true,
				reason: `Path '${pathArg}' is reserved by '${conflict.agent}'. Coordinate via tak mesh/blackboard before editing.`,
			};
		}

		if (workLoop.active && workLoop.strictReservations) {
			const hasOwnReservation = hasOwnedReservationForPath(agentName, targetPath, reservations);
			if (!hasOwnReservation) {
				return {
					block: true,
					reason: `Work loop guard: reserve '${pathArg}' before editing (tak mesh reserve --name ${agentName} --path ${targetPath} --reason task-${workLoop.currentTaskId ?? "current"}).`,
				};
			}
		}
	});
}
