import { existsSync, readdirSync, readFileSync } from "node:fs";
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
import { Container, matchesKey, type SelectItem, SelectList, Text } from "@mariozechner/pi-tui";
import { Type } from "@sinclair/typebox";

type TaskSource = "ready" | "all" | "blocked" | "in_progress" | "mine" | "blackboard" | "inbox";

type Priority = "critical" | "high" | "medium" | "low";
type TaskStatus = "pending" | "in_progress" | "done" | "cancelled";
type VerifyMode = "isolated" | "local";
type WorkClaimStrategy = "priority_then_age" | "epic_closeout";
type WorkCueMode = "editor" | "auto";
type WorkAction = "start" | "stop" | "status";
type LifecycleAction = "start" | "finish" | "handoff" | "cancel" | "reopen" | "unassign";
type GraphAction = "depend" | "undepend" | "reparent" | "orphan";
type TherapistAction = "offline" | "online" | "log";
type MeshAction = "summary" | "send" | "broadcast" | "reserve" | "release" | "feed" | "blockers";
type BlackboardAction = "post" | "show" | "close" | "reopen";
type BlackboardTemplate = "blocker" | "handoff" | "status";

type CommandMode = "pick" | "claim" | "mesh" | "blackboard" | "wait" | "lifecycle" | "graph" | "show" | "help" | "work" | "therapist";

interface TakTask {
	id: string;
	title: string;
	description?: string;
	status: TaskStatus;
	kind: string;
	assignee?: string;
	tags?: string[];
	parent?: string;
	depends_on?: string[];
	planning?: {
		priority?: Priority;
	};
	created_at?: string;
	updated_at?: string;
}

interface BlackboardNote {
	id: string;
	author: string;
	message: string;
	status: "open" | "closed";
	task_ids?: string[];
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

interface VerifyOverlapBlocker {
	agent: string;
	scopePath: string;
	heldPath: string;
	reason?: string;
	ageSeconds: number;
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
	taskId?: string;
	ackInbox?: boolean;
	verifyMode?: VerifyMode;
	workStrategy?: WorkClaimStrategy;
	workCueMode?: WorkCueMode;
}

interface ParsedTakCommand {
	mode: CommandMode;
	filters: TakFilters;
	taskId?: string;
	workAction?: WorkAction;
	meshAction?: MeshAction;
	meshTo?: string;
	meshMessage?: string;
	meshPaths?: string[];
	meshReason?: string;
	meshAll?: boolean;
	blackboardAction?: BlackboardAction;
	blackboardId?: number;
	blackboardMessage?: string;
	blackboardTemplate?: BlackboardTemplate;
	blackboardSinceNote?: number;
	blackboardNoChangeSince?: boolean;
	blackboardBy?: string;
	blackboardReason?: string;
	blackboardTags?: string[];
	blackboardTaskIds?: string[];
	waitPath?: string;
	waitOnTask?: string;
	waitTimeout?: number;
	lifecycleAction?: LifecycleAction;
	lifecycleTaskId?: string;
	lifecycleSummary?: string;
	lifecycleReason?: string;
	lifecycleAssignee?: string;
	graphAction?: GraphAction;
	graphTaskId?: string;
	graphOnTaskIds?: string[];
	graphToTaskId?: string;
	therapistAction?: TherapistAction;
	therapistSession?: string;
	therapistBy?: string;
}

interface WorkLoopState {
	active: boolean;
	tag?: string;
	remaining?: number;
	verifyMode: VerifyMode;
	claimStrategy: WorkClaimStrategy;
	cueMode: WorkCueMode;
	strictReservations: boolean;
	currentTaskId?: string;
	processed: number;
}

interface TakWorkLoopPayload {
	active: boolean;
	tag?: string;
	remaining?: number;
	verifyMode: VerifyMode;
	claimStrategy: WorkClaimStrategy;
	currentTaskId?: string;
	processed: number;
}

interface TakWorkResponsePayload {
	event: string;
	agent: string;
	loop: TakWorkLoopPayload;
	currentTask?: TakTask;
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
/tak work [tag:<tag>] [limit:<n>] [verify:isolated|local] [auto|cue:auto|cue:editor]
                                   Start/resume autonomous work loop
/tak work status|stop              Inspect or stop work loop
/tak mesh [summary|send|broadcast|reserve|release|feed|blockers]
                                   Mesh action parity: summary + coordination subcommands
/tak blackboard [post|show|close|reopen]
                                   Blackboard action parity for note lifecycle operations
/tak wait path:<path>|on-task:<id> [timeout:<sec>]
                                   Deterministic wait for reservation/task unblock conditions
/tak start|finish|handoff|cancel|reopen|unassign <task-id> [...options]
                                   Lifecycle shortcuts for core task transitions
/tak depend|undepend|reparent|orphan <task-id> [...options]
                                   Dependency/structure shortcuts for task graph edits
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
- strategy:<value>  (for /tak work; value = priority_then_age | epic_closeout)
- cue:<mode>        (for /tak work; mode = auto | editor)
- auto              (shorthand for cue:auto)
- session:<id|path>     (for /tak therapist online)
- by:<name>             (for /tak therapist offline|online and blackboard close/reopen)
- to:<agent>            (for /tak mesh send)
- message:<text>        (for /tak mesh send|broadcast and blackboard post; free text also supported)
- path:<path>           (for /tak mesh reserve|release|blockers and /tak wait path target; repeatable)
- on-task:<id>          (for /tak wait task unblock target)
- timeout:<sec>         (for /tak wait timeout)
- on:<id[,id]>          (for /tak depend and /tak undepend dependency targets)
- to:<id>               (for /tak reparent destination)
- summary:<text>        (for /tak handoff summary)
- reason:<text>         (for /tak mesh reserve, /tak cancel, and blackboard close)
- all                   (for /tak mesh release --all)
- template:<kind>       (for /tak blackboard post; blocker|handoff|status)
- since-note:<id>       (for /tak blackboard post delta context)
- no-change-since       (for /tak blackboard post with since-note)

Work mode notes:
- Automatically claims the next available task for you.
- When the current task is finished/handed off/cancelled, the next task is auto-claimed.
- Cue mode defaults to editor prefill; use auto (or cue:auto) to push each claimed task as a user message.
- In work mode, edits are blocked unless the path is reserved by your agent.
- With verify:isolated (default), local build/test/check commands are blocked only when reservation scope overlaps (or when scope is undefined while peers hold reservations).

Picker notes:
- Task picker rows show wider task metadata by default.
- Press d in the task picker to toggle details for the highlighted task.
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
	"mesh summary",
	"mesh send",
	"mesh broadcast",
	"mesh reserve",
	"mesh release",
	"mesh feed",
	"mesh blockers",
	"blackboard post",
	"blackboard show",
	"blackboard close",
	"blackboard reopen",
	"wait",
	"wait path:",
	"wait on-task:",
	"start",
	"finish",
	"handoff",
	"cancel",
	"reopen",
	"unassign",
	"depend",
	"undepend",
	"reparent",
	"orphan",
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
	"to:",
	"message:",
	"path:",
	"on:",
	"on-task:",
	"timeout:",
	"summary:",
	"reason:",
	"template:blocker",
	"template:handoff",
	"template:status",
	"since-note:",
	"no-change-since",
	"all",
	"ack",
	"auto",
	"cue:auto",
	"cue:editor",
	"verify:isolated",
	"verify:local",
	"strategy:priority_then_age",
	"strategy:epic_closeout",
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

const HEARTBEAT_INTERVAL_FALLBACK_MS = 30_000;

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

function isRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function normalizeTaskIdInput(token: string): string | undefined {
	const trimmed = token.trim();
	if (!trimmed) return undefined;
	if (/^\d+$/.test(trimmed)) return trimmed;
	if (/^[0-9a-fA-F]+$/.test(trimmed)) return trimmed.toLowerCase();
	return undefined;
}

function canonicalTaskId(value: unknown): string | undefined {
	if (typeof value === "string") {
		const trimmed = value.trim();
		if (!trimmed) return undefined;
		if (/^[0-9a-fA-F]{16}$/.test(trimmed)) return trimmed.toLowerCase();
		if (/^\d+$/.test(trimmed)) {
			try {
				return BigInt(trimmed).toString(16).padStart(16, "0");
			} catch {
				return undefined;
			}
		}
		return undefined;
	}

	if (typeof value === "number") {
		if (!Number.isFinite(value) || value < 0 || !Number.isInteger(value)) return undefined;
		return BigInt(value).toString(16).padStart(16, "0");
	}

	if (typeof value === "bigint") {
		if (value < 0n) return undefined;
		return value.toString(16).padStart(16, "0");
	}

	return undefined;
}

function compareTaskIds(a: string, b: string): number {
	if (a === b) return 0;
	try {
		const aBig = BigInt(`0x${a}`);
		const bBig = BigInt(`0x${b}`);
		if (aBig < bBig) return -1;
		if (aBig > bBig) return 1;
		return 0;
	} catch {
		return a.localeCompare(b);
	}
}

function toStringArray(value: unknown): string[] {
	if (!Array.isArray(value)) return [];
	return value.filter((item): item is string => typeof item === "string");
}

function toDependencyIdArray(value: unknown): string[] {
	if (!Array.isArray(value)) return [];

	const ids: string[] = [];
	for (const item of value) {
		if (isRecord(item)) {
			const depId = canonicalTaskId(item.id);
			if (depId) ids.push(depId);
			continue;
		}

		const depId = canonicalTaskId(item);
		if (depId) ids.push(depId);
	}

	return ids;
}

function coerceTakTask(value: unknown): TakTask | null {
	if (!isRecord(value)) return null;

	const id = canonicalTaskId(value.id);
	if (!id) return null;

	const title = typeof value.title === "string" ? value.title : "";
	const status =
		value.status === "pending" || value.status === "in_progress" || value.status === "done" || value.status === "cancelled"
			? (value.status as TaskStatus)
			: "pending";
	const kind = typeof value.kind === "string" ? value.kind : "task";

	const description = typeof value.description === "string" ? value.description : undefined;
	const assignee = typeof value.assignee === "string" ? value.assignee : undefined;
	const tags = toStringArray(value.tags);
	const parent = canonicalTaskId(value.parent);
	const depends_on = toDependencyIdArray(value.depends_on);

	let planning: TakTask["planning"] | undefined;
	if (isRecord(value.planning)) {
		const rawPriority = value.planning.priority;
		if (rawPriority === "critical" || rawPriority === "high" || rawPriority === "medium" || rawPriority === "low") {
			planning = { priority: rawPriority };
		}
	}

	return {
		id,
		title,
		description,
		status,
		kind,
		assignee,
		tags,
		parent,
		depends_on,
		planning,
		created_at: typeof value.created_at === "string" ? value.created_at : undefined,
		updated_at: typeof value.updated_at === "string" ? value.updated_at : undefined,
	};
}

function coerceTakTaskArray(value: unknown): TakTask[] {
	if (!Array.isArray(value)) return [];
	return value.map(coerceTakTask).filter((task): task is TakTask => task !== null);
}

function coerceNonNegativeInteger(value: unknown): number | undefined {
	if (typeof value === "number" && Number.isFinite(value) && value >= 0) {
		return Math.trunc(value);
	}
	if (typeof value === "string" && /^\d+$/.test(value)) {
		const parsed = Number.parseInt(value, 10);
		if (Number.isFinite(parsed) && parsed >= 0) return parsed;
	}
	return undefined;
}

function coerceTakWorkLoopPayload(value: unknown): TakWorkLoopPayload | null {
	if (!isRecord(value)) return null;
	if (typeof value.active !== "boolean") return null;

	const verifyMode = value.verify_mode === "local" ? "local" : "isolated";
	const claimStrategy = value.claim_strategy === "epic_closeout" ? "epic_closeout" : "priority_then_age";
	const processed = coerceNonNegativeInteger(value.processed) ?? 0;
	const remainingRaw = coerceNonNegativeInteger(value.remaining);
	const tag = typeof value.tag === "string" && value.tag.trim() ? value.tag.trim() : undefined;

	return {
		active: value.active,
		tag,
		remaining: remainingRaw,
		verifyMode,
		claimStrategy,
		currentTaskId: canonicalTaskId(value.current_task_id),
		processed,
	};
}

function coerceTakWorkResponse(value: unknown): TakWorkResponsePayload | null {
	if (!isRecord(value)) return null;
	const event = typeof value.event === "string" ? value.event : undefined;
	const loop = coerceTakWorkLoopPayload(value.loop);
	if (!event || !loop) return null;

	const currentTask = value.current_task === null ? undefined : coerceTakTask(value.current_task);

	return {
		event,
		agent: typeof value.agent === "string" ? value.agent : "",
		loop,
		currentTask: currentTask ?? undefined,
	};
}

function coerceBlackboardNote(value: unknown): BlackboardNote | null {
	if (!isRecord(value)) return null;
	const rawId = value.id;
	const id =
		typeof rawId === "string"
			? rawId
			: typeof rawId === "number" && Number.isFinite(rawId)
				? String(Math.trunc(rawId))
				: undefined;
	if (!id) return null;

	const taskIds = Array.isArray(value.task_ids)
		? value.task_ids
				.map((taskId) => canonicalTaskId(taskId))
				.filter((taskId): taskId is string => Boolean(taskId))
		: undefined;

	const tags = toStringArray(value.tags);

	return {
		id,
		author: typeof value.author === "string" ? value.author : "unknown",
		message: typeof value.message === "string" ? value.message : "",
		status: value.status === "closed" ? "closed" : "open",
		task_ids: taskIds,
		tags,
		updated_at: typeof value.updated_at === "string" ? value.updated_at : undefined,
	};
}

function coerceBlackboardNoteArray(value: unknown): BlackboardNote[] {
	if (!Array.isArray(value)) return [];
	return value.map(coerceBlackboardNote).filter((note): note is BlackboardNote => note !== null);
}

function isDigit(ch: string): boolean {
	return ch >= "0" && ch <= "9";
}

function quoteIntegerLiterals(jsonText: string): string {
	let out = "";
	let i = 0;
	let inString = false;
	let escaping = false;

	while (i < jsonText.length) {
		const ch = jsonText[i]!;

		if (inString) {
			out += ch;
			if (escaping) {
				escaping = false;
			} else if (ch === "\\") {
				escaping = true;
			} else if (ch === '"') {
				inString = false;
			}
			i += 1;
			continue;
		}

		if (ch === '"') {
			inString = true;
			out += ch;
			i += 1;
			continue;
		}

		if (ch === "-" || isDigit(ch)) {
			let j = i;
			if (jsonText[j] === "-") j += 1;
			let sawDigit = false;
			while (j < jsonText.length && isDigit(jsonText[j]!)) {
				sawDigit = true;
				j += 1;
			}
			if (!sawDigit) {
				out += ch;
				i += 1;
				continue;
			}

			let integerOnly = true;
			if (jsonText[j] === ".") {
				integerOnly = false;
				j += 1;
				while (j < jsonText.length && isDigit(jsonText[j]!)) j += 1;
			}
			if (jsonText[j] === "e" || jsonText[j] === "E") {
				integerOnly = false;
				j += 1;
				if (jsonText[j] === "+" || jsonText[j] === "-") j += 1;
				while (j < jsonText.length && isDigit(jsonText[j]!)) j += 1;
			}

			const token = jsonText.slice(i, j);
			out += integerOnly ? `"${token}"` : token;
			i = j;
			continue;
		}

		out += ch;
		i += 1;
	}

	return out;
}

function parseTakJson(jsonText: string): unknown {
	return JSON.parse(quoteIntegerLiterals(jsonText));
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

	const rawFirst = tokens[0]!;
	const first = rawFirst.toLowerCase();

	const firstTaskId = normalizeTaskIdInput(rawFirst);
	if (firstTaskId) {
		return {
			mode: "show",
			filters,
			taskId: firstTaskId,
		};
	}

	if (first === "help") {
		return { mode: "help", filters };
	}

	if (first === "mesh") {
		let meshAction: MeshAction = "summary";
		let tokenStart = 1;
		const second = tokens[1]?.toLowerCase();
		if (
			second === "summary" ||
			second === "send" ||
			second === "broadcast" ||
			second === "reserve" ||
			second === "release" ||
			second === "feed" ||
			second === "blockers"
		) {
			meshAction = second;
			tokenStart = 2;
		}

		let meshTo: string | undefined;
		let meshReason: string | undefined;
		let meshAll = false;
		const meshPaths: string[] = [];
		const messageParts: string[] = [];

		for (const token of tokens.slice(tokenStart)) {
			const [rawKey, ...rawValueParts] = token.split(":");
			if (rawKey && rawValueParts.length > 0) {
				const key = rawKey.toLowerCase();
				const value = rawValueParts.join(":").trim();

				if ((key === "to" || key === "agent") && value) {
					meshTo = value;
					continue;
				}
				if ((key === "message" || key === "msg" || key === "text") && value) {
					messageParts.push(value);
					continue;
				}
				if (key === "path" && value) {
					meshPaths.push(value);
					continue;
				}
				if (key === "reason" && value) {
					meshReason = value;
					continue;
				}

				applyFilterToken(token, filters);
				continue;
			}

			if (token.toLowerCase() === "all") {
				meshAll = true;
				continue;
			}

			if (meshAction === "reserve" || meshAction === "release" || meshAction === "blockers") {
				meshPaths.push(token);
			} else {
				messageParts.push(token);
			}
		}

		const meshMessage = messageParts.join(" ").trim() || undefined;
		const uniqueMeshPaths = Array.from(new Set(meshPaths.map((p) => p.trim()).filter(Boolean)));

		return {
			mode: "mesh",
			filters,
			meshAction,
			meshTo,
			meshMessage,
			meshPaths: uniqueMeshPaths,
			meshReason,
			meshAll,
		};
	}

	if (first === "blackboard") {
		const second = tokens[1]?.toLowerCase();
		if (second === "post" || second === "show" || second === "close" || second === "reopen") {
			const blackboardAction: BlackboardAction = second;
			let blackboardId: number | undefined;
			let blackboardTemplate: BlackboardTemplate | undefined;
			let blackboardSinceNote: number | undefined;
			let blackboardNoChangeSince = false;
			let blackboardBy: string | undefined;
			let blackboardReason: string | undefined;
			const blackboardTags: string[] = [];
			const blackboardTaskIds: string[] = [];
			const messageParts: string[] = [];

			for (const token of tokens.slice(2)) {
				const lower = token.toLowerCase();
				if (lower === "no-change-since") {
					blackboardNoChangeSince = true;
					continue;
				}

				const [rawKey, ...rawValueParts] = token.split(":");
				if (rawKey && rawValueParts.length > 0) {
					const key = rawKey.toLowerCase();
					const value = rawValueParts.join(":").trim();
					if (!value) continue;

					if ((key === "by" || key === "from") && value) {
						blackboardBy = value;
						continue;
					}
					if ((key === "message" || key === "msg" || key === "text") && value) {
						messageParts.push(value);
						continue;
					}
					if (key === "template") {
						const normalized = value.toLowerCase();
						if (normalized === "blocker" || normalized === "handoff" || normalized === "status") {
							blackboardTemplate = normalized;
						}
						continue;
					}
					if (key === "since-note" || key === "since_note" || key === "since") {
						if (/^\d+$/.test(value)) {
							blackboardSinceNote = Number.parseInt(value, 10);
						}
						continue;
					}
					if (key === "reason") {
						blackboardReason = value;
						continue;
					}
					if (key === "tag") {
						blackboardTags.push(value);
						continue;
					}
					if (key === "task") {
						const taskId = normalizeTaskIdInput(value);
						if (taskId) {
							blackboardTaskIds.push(taskId);
						}
						continue;
					}
					if ((key === "id" || key === "note") && /^\d+$/.test(value)) {
						blackboardId = Number.parseInt(value, 10);
						continue;
					}

					applyFilterToken(token, filters);
					continue;
				}

				if ((blackboardAction === "show" || blackboardAction === "close" || blackboardAction === "reopen") && /^\d+$/.test(token)) {
					blackboardId = Number.parseInt(token, 10);
					continue;
				}

				if (blackboardAction === "post") {
					messageParts.push(token);
				}
			}

			const dedupedTags = Array.from(new Set(blackboardTags.map((tag) => tag.trim()).filter(Boolean)));
			const dedupedTaskIds = Array.from(new Set(blackboardTaskIds.map((taskId) => taskId.trim()).filter(Boolean)));
			const blackboardMessage = messageParts.join(" ").trim() || undefined;

			return {
				mode: "blackboard",
				filters,
				blackboardAction,
				blackboardId,
				blackboardMessage,
				blackboardTemplate,
				blackboardSinceNote,
				blackboardNoChangeSince,
				blackboardBy,
				blackboardReason,
				blackboardTags: dedupedTags,
				blackboardTaskIds: dedupedTaskIds,
			};
		}
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

	if (first === "wait") {
		let waitPath: string | undefined;
		let waitOnTask: string | undefined;
		let waitTimeout: number | undefined;

		for (const token of tokens.slice(1)) {
			const [rawKey, ...rawValueParts] = token.split(":");
			if (rawKey && rawValueParts.length > 0) {
				const key = rawKey.toLowerCase();
				const value = rawValueParts.join(":").trim();
				if (!value) continue;

				if (key === "path") {
					waitPath = value;
					continue;
				}
				if (key === "on-task" || key === "on_task" || key === "task") {
					const parsed = normalizeTaskIdInput(value);
					if (parsed) waitOnTask = parsed;
					continue;
				}
				if (key === "timeout") {
					const parsed = Number.parseInt(value, 10);
					if (Number.isFinite(parsed) && parsed > 0) waitTimeout = parsed;
					continue;
				}
			}

			if (!waitPath && !waitOnTask) {
				const maybeTask = normalizeTaskIdInput(token);
				if (maybeTask) {
					waitOnTask = maybeTask;
					continue;
				}
				waitPath = token;
			}
		}

		return { mode: "wait", filters, waitPath, waitOnTask, waitTimeout };
	}

	if (
		first === "start" ||
		first === "finish" ||
		first === "handoff" ||
		first === "cancel" ||
		first === "reopen" ||
		first === "unassign"
	) {
		const lifecycleAction: LifecycleAction = first;
		let lifecycleTaskId: string | undefined;
		let lifecycleAssignee: string | undefined;
		const summaryParts: string[] = [];
		const reasonParts: string[] = [];

		for (const token of tokens.slice(1)) {
			const [rawKey, ...rawValueParts] = token.split(":");
			if (rawKey && rawValueParts.length > 0) {
				const key = rawKey.toLowerCase();
				const value = rawValueParts.join(":").trim();
				if (!value) continue;

				if (key === "task" || key === "id") {
					const parsed = normalizeTaskIdInput(value);
					if (parsed) lifecycleTaskId = parsed;
					continue;
				}
				if (key === "assignee" || key === "by" || key === "from") {
					lifecycleAssignee = value;
					continue;
				}
				if (key === "summary") {
					summaryParts.push(value);
					continue;
				}
				if (key === "reason") {
					reasonParts.push(value);
					continue;
				}

				applyFilterToken(token, filters);
				continue;
			}

			const maybeTask = normalizeTaskIdInput(token);
			if (!lifecycleTaskId && maybeTask) {
				lifecycleTaskId = maybeTask;
				continue;
			}

			if (lifecycleAction === "handoff") {
				summaryParts.push(token);
			} else if (lifecycleAction === "cancel") {
				reasonParts.push(token);
			}
		}

		const lifecycleSummary = summaryParts.join(" ").trim() || undefined;
		const lifecycleReason = reasonParts.join(" ").trim() || undefined;

		return {
			mode: "lifecycle",
			filters,
			lifecycleAction,
			lifecycleTaskId,
			lifecycleSummary,
			lifecycleReason,
			lifecycleAssignee,
		};
	}

	if (first === "depend" || first === "undepend" || first === "reparent" || first === "orphan") {
		const graphAction: GraphAction = first;
		let graphTaskId: string | undefined;
		let graphToTaskId: string | undefined;
		const graphOnTaskIds: string[] = [];

		const pushGraphTargets = (raw: string) => {
			for (const piece of raw.split(",")) {
				const parsed = normalizeTaskIdInput(piece.trim());
				if (parsed) graphOnTaskIds.push(parsed);
			}
		};

		for (const token of tokens.slice(1)) {
			const [rawKey, ...rawValueParts] = token.split(":");
			if (rawKey && rawValueParts.length > 0) {
				const key = rawKey.toLowerCase();
				const value = rawValueParts.join(":").trim();
				if (!value) continue;

				if (key === "task" || key === "id") {
					const parsed = normalizeTaskIdInput(value);
					if (parsed) graphTaskId = parsed;
					continue;
				}
				if (key === "to") {
					const parsed = normalizeTaskIdInput(value);
					if (parsed) graphToTaskId = parsed;
					continue;
				}
				if (key === "on") {
					pushGraphTargets(value);
					continue;
				}

				applyFilterToken(token, filters);
				continue;
			}

			const maybeTask = normalizeTaskIdInput(token);
			if (!graphTaskId && maybeTask) {
				graphTaskId = maybeTask;
				continue;
			}

			if (graphAction === "reparent" && !graphToTaskId && maybeTask) {
				graphToTaskId = maybeTask;
				continue;
			}

			if (graphAction === "depend" || graphAction === "undepend") {
				pushGraphTargets(token);
			}
		}

		const uniqueOnTaskIds = Array.from(new Set(graphOnTaskIds));
		return {
			mode: "graph",
			filters,
			graphAction,
			graphTaskId,
			graphOnTaskIds: uniqueOnTaskIds,
			graphToTaskId,
		};
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

	if (normalized === "auto") {
		filters.workCueMode = "auto";
		return;
	}

	if (normalized === "editor" || normalized === "prompt") {
		filters.workCueMode = "editor";
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
			const parsed = normalizeTaskIdInput(value);
			if (parsed) filters.taskId = parsed;
			break;
		}
		case "verify":
			if (value.toLowerCase() === "isolated" || value.toLowerCase() === "local") {
				filters.verifyMode = value.toLowerCase() as VerifyMode;
			}
			break;
		case "strategy":
			if (value.toLowerCase() === "priority_then_age" || value.toLowerCase() === "epic_closeout") {
				filters.workStrategy = value.toLowerCase() as WorkClaimStrategy;
			}
			break;
		case "cue":
			if (value.toLowerCase() === "auto") {
				filters.workCueMode = "auto";
			} else if (value.toLowerCase() === "editor" || value.toLowerCase() === "prompt") {
				filters.workCueMode = "editor";
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

		return compareTaskIds(a.id, b.id);
	});
}

function taskAgeLabel(task: TakTask): string {
	const ts = parseTimestamp(task.created_at);
	if (!Number.isFinite(ts) || ts === Number.MAX_SAFE_INTEGER) return "age:?";
	const days = Math.floor((Date.now() - ts) / (1000 * 60 * 60 * 24));
	if (days <= 0) return "age:<1d";
	return `age:${days}d`;
}

function taskStatusGlyph(status: TaskStatus): string {
	switch (status) {
		case "pending":
			return "○";
		case "in_progress":
			return "▶";
		case "done":
			return "✓";
		case "cancelled":
			return "✕";
		default:
			return "•";
	}
}

function taskPriorityLabel(task: TakTask): string {
	return task.planning?.priority ?? "unprioritized";
}

interface TaskPickerItem extends SelectItem {
	task: TakTask;
	noteCount: number;
}

function buildTaskPickerItems(tasks: TakTask[], noteCountByTask: Map<string, number>): TaskPickerItem[] {
	return tasks.map((task) => {
		const noteCount = noteCountByTask.get(task.id) ?? 0;
		const notePart = noteCount > 0 ? ` • bb:${noteCount}` : "";
		const assigneePart = task.assignee ? ` • @${task.assignee}` : " • unassigned";
		return {
			value: String(task.id),
			label: `${taskStatusGlyph(task.status)} #${task.id}`,
			description: `${task.title} • ${taskPriorityLabel(task)} • ${taskAgeLabel(task)}${assigneePart}${notePart}`,
			task,
			noteCount,
		};
	});
}

function formatTaskPickerDetail(task: TakTask, noteCount: number): string {
	const tags = task.tags?.length ? task.tags.join(", ") : "-";
	const dependsOn = task.depends_on?.length ? task.depends_on.join(", ") : "-";
	const description = task.description?.trim() ? truncateText(task.description, 700) : "(no description)";

	const lines = [
		`#${task.id} ${task.title}`,
		`status: ${task.status} | kind: ${task.kind} | priority: ${taskPriorityLabel(task)}`,
		`assignee: ${task.assignee ?? "unassigned"} | ${taskAgeLabel(task)} | blackboard:${noteCount}`,
		`tags: ${tags}`,
		`depends_on: ${dependsOn}`,
	];

	if (task.parent) {
		lines.push(`parent: ${task.parent}`);
	}

	lines.push("", "description:", description);
	return lines.join("\n");
}

async function pickTaskFromList(
	ctx: ExtensionContext,
	title: string,
	tasks: TakTask[],
	noteCountByTask: Map<string, number>,
): Promise<string | null> {
	const taskItems = buildTaskPickerItems(tasks, noteCountByTask);
	if (taskItems.length === 0) return null;

	return ctx.ui.custom<string | null>((tui, theme, _kb, done) => {
		const container = new Container();
		container.addChild(new DynamicBorder((s: string) => theme.fg("accent", s)));
		container.addChild(new Text(theme.fg("accent", theme.bold(title))));

		const list = new SelectList(taskItems, Math.min(Math.max(taskItems.length, 6), 18), {
			selectedPrefix: (t) => theme.fg("accent", t),
			selectedText: (t) => theme.fg("accent", t),
			description: (t) => theme.fg("muted", t),
			scrollInfo: (t) => theme.fg("dim", t),
			noMatch: (t) => theme.fg("warning", t),
		});

		const detailHeader = new Text("", 0, 0);
		const detailBody = new Text("", 0, 0);
		const footer = new Text("", 0, 0);
		let detailsVisible = false;

		const selectedTaskItem = () => {
			const selected = list.getSelectedItem();
			if (!selected) return null;
			return taskItems.find((item) => item.value === selected.value) ?? null;
		};

		const updateDetails = () => {
			if (!detailsVisible) {
				detailHeader.setText("");
				detailBody.setText("");
				return;
			}

			const selected = selectedTaskItem();
			if (!selected) {
				detailHeader.setText(theme.fg("warning", "No task selected."));
				detailBody.setText("");
				return;
			}

			detailHeader.setText(theme.fg("accent", theme.bold("Task detail")));
			detailBody.setText(theme.fg("muted", formatTaskPickerDetail(selected.task, selected.noteCount)));
		};

		const updateFooter = () => {
			footer.setText(
				theme.fg(
					"dim",
					`↑↓ navigate • enter select • d ${detailsVisible ? "hide" : "show"} detail • esc cancel`,
				),
			);
		};

		list.onSelect = (item) => done(String(item.value));
		list.onCancel = () => done(null);
		list.onSelectionChange = () => {
			updateDetails();
			tui.requestRender();
		};

		container.addChild(list);
		container.addChild(detailHeader);
		container.addChild(detailBody);
		container.addChild(footer);
		container.addChild(new DynamicBorder((s: string) => theme.fg("accent", s)));
		updateFooter();

		return {
			render: (w: number) => container.render(w),
			invalidate: () => {
				container.invalidate();
				updateDetails();
				updateFooter();
			},
			handleInput(data: string) {
				if (matchesKey(data, "d") || data === "D") {
					detailsVisible = !detailsVisible;
					updateDetails();
					updateFooter();
					tui.requestRender();
					return;
				}

				list.handleInput(data);
				updateDetails();
				updateFooter();
				tui.requestRender();
			},
		};
	});
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
			parsed = parseTakJson(stdout);
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

function loadActiveAgentNames(cwd: string): Set<string> {
	const registryDir = join(cwd, ".tak", "runtime", "mesh", "registry");
	if (!existsSync(registryDir)) return new Set<string>();

	const active = new Set<string>();
	try {
		for (const entry of readdirSync(registryDir, { withFileTypes: true })) {
			if (!entry.isFile() || !entry.name.endsWith(".json")) continue;
			const path = join(registryDir, entry.name);
			const fallbackName = entry.name.replace(/\.json$/i, "");
			try {
				const content = readFileSync(path, "utf-8");
				const parsed = JSON.parse(content) as { name?: string; status?: string };
				const name = parsed.name?.trim() || fallbackName;
				const status = parsed.status?.trim().toLowerCase();
				if (name && (!status || status === "active")) {
					active.add(name);
				}
			} catch {
				// Ignore malformed records and continue with remaining entries.
			}
		}
	} catch {
		return new Set<string>();
	}

	return active;
}

function filterReservationsToActiveAgents(
	reservations: MeshReservation[],
	activeAgentNames: Set<string>,
): MeshReservation[] {
	if (activeAgentNames.size === 0) return reservations;
	return reservations.filter((reservation) => activeAgentNames.has(reservation.agent));
}

function reservationsOwnedBy(agentName: string, reservations: MeshReservation[]): MeshReservation[] {
	return reservations.filter((reservation) => reservation.agent === agentName);
}

function reservationsForeignTo(agentName: string, reservations: MeshReservation[]): MeshReservation[] {
	return reservations.filter((reservation) => reservation.agent !== agentName);
}

function hasOwnedReservationForPath(agentName: string, targetPath: string, reservations: MeshReservation[]): boolean {
	return reservationsOwnedBy(agentName, reservations).some((reservation) =>
		reservation.paths.some((reservedPath) => pathsConflict(targetPath, reservedPath)),
	);
}

function uniqueNormalizedPaths(paths: string[]): string[] {
	const seen = new Set<string>();
	const ordered: string[] = [];
	for (const path of paths) {
		const normalized = normalizePathLikeTak(path);
		if (!normalized || seen.has(normalized)) continue;
		seen.add(normalized);
		ordered.push(normalized);
	}
	return ordered;
}

function deriveVerifyScopePaths(agentName: string, reservations: MeshReservation[]): string[] {
	const ownedPaths = reservationsOwnedBy(agentName, reservations).flatMap((reservation) => reservation.paths);
	return uniqueNormalizedPaths(ownedPaths);
}

function reservationAgeSeconds(reservation: MeshReservation): number {
	const parsed = Date.parse(reservation.since);
	if (!Number.isFinite(parsed)) return 0;
	return Math.max(0, Math.floor((Date.now() - parsed) / 1000));
}

function findVerifyOverlapBlockers(
	scopePaths: string[],
	foreignReservations: MeshReservation[],
): VerifyOverlapBlocker[] {
	const blockers: VerifyOverlapBlocker[] = [];
	for (const scopePath of scopePaths) {
		for (const reservation of foreignReservations) {
			for (const heldPath of reservation.paths) {
				if (!pathsConflict(scopePath, heldPath)) continue;
				blockers.push({
					agent: reservation.agent,
					scopePath,
					heldPath: normalizePathLikeTak(heldPath),
					reason: reservation.reason,
					ageSeconds: reservationAgeSeconds(reservation),
				});
			}
		}
	}

	blockers.sort((a, b) => {
		if (a.agent !== b.agent) return a.agent.localeCompare(b.agent);
		if (a.scopePath !== b.scopePath) return a.scopePath.localeCompare(b.scopePath);
		return a.heldPath.localeCompare(b.heldPath);
	});

	const deduped: VerifyOverlapBlocker[] = [];
	const seen = new Set<string>();
	for (const blocker of blockers) {
		const key = `${blocker.agent}|${blocker.scopePath}|${blocker.heldPath}`;
		if (seen.has(key)) continue;
		seen.add(key);
		deduped.push(blocker);
	}
	return deduped;
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

function formatVerifyGuardReason(
	agentName: string,
	scopePaths: string[],
	foreignReservations: MeshReservation[],
	blockers: VerifyOverlapBlocker[],
): string {
	if (scopePaths.length === 0) {
		const hintPath = foreignReservations[0]?.paths[0];
		const waitHint = hintPath
			? `Queue/window option: tak wait --path ${normalizePathLikeTak(hintPath)} --timeout 120.`
			: "Queue/window option: tak wait --path <path> --timeout 120.";
		return [
			"Work loop guard (verify:isolated): verification scope is empty while peers hold reservations.",
			`Reserve your verify scope first (tak mesh reserve --name ${agentName} --path <path> --reason task-current).`,
			waitHint,
			"Or switch to verify:local.",
		].join(" ");
	}

	const first = blockers[0]!;
	const reason = first.reason ?? "none";
	const waitHint = `Queue/window option: tak wait --path ${first.heldPath} --timeout 120.`;
	const details = blockers
		.slice(0, 3)
		.map((blocker) => `${blocker.agent}:${blocker.scopePath}↔${blocker.heldPath}`)
		.join(", ");
	const extra = blockers.length > 3 ? ` (+${blockers.length - 3} more)` : "";

	return [
		`Work loop guard (verify:isolated): overlapping reservation scope with '${first.agent}' (scope='${first.scopePath}', held='${first.heldPath}', reason=${reason}, age=${first.ageSeconds}s).`,
		`Overlaps: ${details}${extra}.`,
		waitHint,
		"Or switch to verify:local.",
	].join(" ");
}

function formatWorkLoopStatus(workLoop: WorkLoopState): string {
	if (!workLoop.active) return "work: inactive";
	const parts = ["work: active"];
	if (workLoop.currentTaskId) parts.push(`task=#${workLoop.currentTaskId}`);
	if (workLoop.tag) parts.push(`tag=${workLoop.tag}`);
	if (workLoop.remaining !== undefined) parts.push(`remaining=${workLoop.remaining}`);
	parts.push(`verify=${workLoop.verifyMode}`);
	parts.push(`strategy=${workLoop.claimStrategy}`);
	parts.push(`cue=${workLoop.cueMode}`);
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
			`Work loop active (verify=${workLoop.verifyMode}, strategy=${workLoop.claimStrategy}, cue=${workLoop.cueMode}).`,
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
	let lastHeartbeatAt = 0;
	const seenInboxMessageIds = new Set<string>();
	let workLoop: WorkLoopState = {
		active: false,
		verifyMode: "isolated",
		claimStrategy: "priority_then_age",
		cueMode: "editor",
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
			claimStrategy: "priority_then_age",
			cueMode: "editor",
			strictReservations: true,
			processed: 0,
			...overrides,
		};
	}

	async function maybeHeartbeat(ctx: ExtensionContext): Promise<void> {
		if (!integrationEnabled() || !meshJoined || !agentName) return;

		const now = Date.now();
		if (now - lastHeartbeatAt < HEARTBEAT_INTERVAL_FALLBACK_MS) {
			return;
		}

		lastHeartbeatAt = now;
		const heartbeatResult = await runTak(pi, ["mesh", "heartbeat", "--name", agentName], {
			json: false,
			timeoutMs: 10000,
		});
		if (!heartbeatResult.ok) {
			ctx.ui.notify(
				heartbeatResult.errorMessage ?? "tak mesh heartbeat failed",
				"warning",
			);
		}
	}

	function cueTaskForWorkLoop(ctx: ExtensionContext, task: TakTask, notes: BlackboardNote[]): void {
		const cueText = buildTaskEditorText(task, agentName, notes, { workLoop });

		if (workLoop.cueMode === "auto") {
			try {
				if (ctx.isIdle()) {
					pi.sendUserMessage(cueText);
				} else {
					pi.sendUserMessage(cueText, { deliverAs: "followUp" });
				}
				return;
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				ctx.ui.notify(`Work-loop auto cue failed, using editor prefill instead: ${message}`, "warning");
			}
		}

		ctx.ui.setEditorText(cueText);
	}

	function applyWorkLoopFromRuntime(
		runtime: TakWorkLoopPayload,
		overrides?: { cueMode?: WorkCueMode; strictReservations?: boolean },
	): void {
		workLoop = {
			...workLoop,
			active: runtime.active,
			tag: runtime.tag,
			remaining: runtime.remaining,
			verifyMode: runtime.verifyMode,
			claimStrategy: runtime.claimStrategy,
			currentTaskId: runtime.currentTaskId,
			processed: runtime.processed,
			cueMode: overrides?.cueMode ?? workLoop.cueMode,
			strictReservations: overrides?.strictReservations ?? workLoop.strictReservations,
		};
	}

	async function fetchOpenTaskNotes(taskId: string): Promise<BlackboardNote[]> {
		const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", taskId]);
		return coerceBlackboardNoteArray(notesResult.parsed);
	}

	async function runTakWorkAction(
		ctx: ExtensionContext,
		action: WorkAction,
		options?: { tag?: string; limit?: number; verifyMode?: VerifyMode; strategy?: WorkClaimStrategy },
	): Promise<TakWorkResponsePayload | null> {
		if (!agentName) {
			ctx.ui.notify("/tak work requires mesh agent identity", "warning");
			return null;
		}

		const workArgs = ["work"];
		if (action !== "start") workArgs.push(action);
		workArgs.push("--assignee", agentName);

		if (action === "start") {
			if (options?.tag) workArgs.push("--tag", options.tag);
			if (options?.limit !== undefined) workArgs.push("--limit", String(options.limit));
			if (options?.verifyMode) workArgs.push("--verify", options.verifyMode);
			if (options?.strategy) workArgs.push("--strategy", options.strategy);
		}

		const result = await runTak(pi, workArgs);
		if (!result.ok) {
			ctx.ui.notify(result.errorMessage ?? `tak work ${action} failed`, "warning");
			return null;
		}

		const response = coerceTakWorkResponse(result.parsed);
		if (!response) {
			ctx.ui.notify(`tak work ${action} returned an unexpected payload`, "warning");
			return null;
		}

		return response;
	}

	async function handleTakWorkResponse(
		ctx: ExtensionContext,
		response: TakWorkResponsePayload,
		options?: { cueMode?: WorkCueMode; notifyTransitions?: boolean },
	): Promise<void> {
		const previousTaskId = workLoop.currentTaskId;
		applyWorkLoopFromRuntime(response.loop, {
			cueMode: options?.cueMode,
			strictReservations: true,
		});

		const task = response.currentTask;
		const shouldCueTask =
			Boolean(task) &&
			(response.event === "claimed" ||
				response.event === "attached" ||
				(response.event === "continued" && task?.id !== previousTaskId));
		if (task && shouldCueTask) {
			const notes = await fetchOpenTaskNotes(task.id);
			cueTaskForWorkLoop(ctx, task, notes);
		}

		if (!options?.notifyTransitions) return;

		switch (response.event) {
			case "claimed":
				if (task) ctx.ui.notify(`Work loop claimed task #${task.id}: ${task.title}`, "info");
				break;
			case "attached":
				if (task) ctx.ui.notify(`Work loop attached to in-progress task #${task.id}: ${task.title}`, "info");
				break;
			case "limit_reached":
				ctx.ui.notify("Work loop finished requested task limit.", "info");
				break;
			case "no_work":
				ctx.ui.notify("Work loop stopped: no claimable task available.", "info");
				break;
		}
	}

	async function syncWorkLoop(ctx: ExtensionContext): Promise<void> {
		if (!integrationEnabled() || !workLoop.active || !agentName) return;
		const response = await runTakWorkAction(ctx, "start");
		if (!response) return;
		await handleTakWorkResponse(ctx, response, { notifyTransitions: true });
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

		const readyTasks = sortTasksUrgentThenOldest(coerceTakTaskArray(readyResult.parsed));
		const blockedTasks = sortTasksUrgentThenOldest(coerceTakTaskArray(blockedResult.parsed));
		const inProgressTasks = sortTasksUrgentThenOldest(coerceTakTaskArray(inProgressResult.parsed));
		const openNotes = coerceBlackboardNoteArray(blackboardResult.parsed);

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
					const response = await runTakWorkAction(ctx, "status");
					if (!response) {
						await refreshStatus(ctx);
						return;
					}
					await handleTakWorkResponse(ctx, response);
					const lines = [formatWorkLoopStatus(workLoop)];
					if (response.currentTask) {
						lines.push(`current: #${response.currentTask.id} ${response.currentTask.title}`);
					}
					ctx.ui.setEditorText(lines.join("\n"));
					ctx.ui.notify("Inserted /tak work status", "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "stop") {
					const hadBeenActive = workLoop.active;
					const response = await runTakWorkAction(ctx, "stop");
					if (response) {
						await handleTakWorkResponse(ctx, response);
					} else {
						resetWorkLoop();
					}
					ctx.ui.notify(hadBeenActive ? "Stopped /tak work loop" : "Work loop already inactive", "info");
					await refreshStatus(ctx);
					return;
				}

				const cueMode = parsed.filters.workCueMode ?? workLoop.cueMode ?? "editor";
				const response = await runTakWorkAction(ctx, "start", {
					tag: parsed.filters.tag,
					limit: parsed.filters.limit,
					verifyMode: parsed.filters.verifyMode,
					strategy: parsed.filters.workStrategy,
				});
				if (!response) {
					await refreshStatus(ctx);
					return;
				}

				await handleTakWorkResponse(ctx, response, {
					cueMode,
					notifyTransitions: true,
				});
				if (workLoop.active) {
					ctx.ui.notify(`Started /tak work loop (${formatWorkLoopStatus(workLoop)})`, "info");
				}
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "wait") {
				const waitArgs = ["wait"];
				if (parsed.waitPath) {
					waitArgs.push("--path", parsed.waitPath);
				} else if (parsed.waitOnTask) {
					waitArgs.push("--on-task", parsed.waitOnTask);
				} else {
					ctx.ui.notify("Usage: /tak wait path:<path> | on-task:<id> [timeout:<sec>]", "warning");
					return;
				}

				if (parsed.waitTimeout !== undefined) {
					waitArgs.push("--timeout", String(parsed.waitTimeout));
				}

				const effectiveTimeoutSecs = parsed.waitTimeout ?? 120;
				const waitResult = await runTak(pi, waitArgs, {
					timeoutMs: Math.max(15000, (effectiveTimeoutSecs + 2) * 1000),
				});

				if (!waitResult.ok) {
					const target = parsed.waitPath ? `path:${parsed.waitPath}` : `on-task:${parsed.waitOnTask ?? "?"}`;
					ctx.ui.setEditorText(
						[
							"# tak wait",
							`target: ${target}`,
							`result: timeout_or_error`,
							"",
							waitResult.errorMessage ?? "tak wait failed",
							"",
							"Tips:",
							parsed.waitPath
								? `- Diagnose blockers: /tak mesh blockers path:${parsed.waitPath}`
								: `- Inspect task + deps: /tak ${parsed.waitOnTask ?? "<task-id>"}`,
							"- Retry with a larger timeout:<sec> if needed.",
						].join("\n"),
					);
					ctx.ui.notify(waitResult.errorMessage ?? "tak wait failed", "warning");
					await refreshStatus(ctx);
					return;
				}

				let mode = parsed.waitPath ? "path" : "task";
				let target = parsed.waitPath ?? parsed.waitOnTask ?? "-";
				let waitedMs: number | undefined;
				if (isRecord(waitResult.parsed)) {
					if (typeof waitResult.parsed.mode === "string") {
						mode = waitResult.parsed.mode;
					}
					if (mode === "path" && typeof waitResult.parsed.path === "string") {
						target = waitResult.parsed.path;
					}
					if (mode === "task") {
						const taskTarget = canonicalTaskId(waitResult.parsed.task_id) ??
							(typeof waitResult.parsed.task_id === "string" ? waitResult.parsed.task_id : undefined);
						if (taskTarget) target = taskTarget;
					}
					waitedMs = coerceNonNegativeInteger(waitResult.parsed.waited_ms);
				}

				const lines = [
					"# tak wait",
					`mode: ${mode}`,
					`target: ${target}`,
					`status: ready`,
					`waited_ms: ${waitedMs ?? "unknown"}`,
					"",
					"Next:",
					mode === "path"
						? "- Retry the previously blocked action now that the reservation conflict cleared."
						: "- Continue with /tak work or reload the task to proceed.",
				];
				ctx.ui.setEditorText(lines.join("\n"));
				ctx.ui.notify("Wait condition satisfied", "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "lifecycle") {
				const action = parsed.lifecycleAction;
				if (!action) {
					ctx.ui.notify("Invalid lifecycle action", "warning");
					return;
				}

				let taskId = parsed.lifecycleTaskId;
				if (!taskId && agentName && (action === "finish" || action === "handoff" || action === "cancel" || action === "unassign")) {
					const mineResult = await runTak(pi, ["list", "--status", "in_progress", "--assignee", agentName]);
					const mine = sortTasksUrgentThenOldest(coerceTakTaskArray(mineResult.parsed));
					if (mine.length === 1) {
						taskId = mine[0]!.id;
					} else if (mine.length > 1) {
						ctx.ui.notify("Multiple in-progress tasks found; specify task id explicitly.", "warning");
						return;
					}
				}

				if (!taskId) {
					ctx.ui.notify(`Usage: /tak ${action} <task-id>`, "warning");
					return;
				}

				const lifecycleArgs = [action, taskId];
				if (action === "start") {
					const assignee = parsed.lifecycleAssignee ?? agentName;
					if (assignee) {
						lifecycleArgs.push("--assignee", assignee);
					}
				} else if (action === "handoff") {
					const summary = parsed.lifecycleSummary?.trim();
					if (!summary) {
						ctx.ui.notify("Usage: /tak handoff <task-id> summary:<text>", "warning");
						return;
					}
					lifecycleArgs.push("--summary", summary);
				} else if (action === "cancel") {
					const reason = parsed.lifecycleReason?.trim();
					if (reason) {
						lifecycleArgs.push("--reason", reason);
					}
				}

				const lifecycleResult = await runTak(pi, lifecycleArgs);
				if (!lifecycleResult.ok) {
					ctx.ui.notify(lifecycleResult.errorMessage ?? `tak ${action} failed`, "error");
					return;
				}

				const task = coerceTakTask(lifecycleResult.parsed);
				if (task) {
					const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", task.id]);
					const notes = coerceBlackboardNoteArray(notesResult.parsed);
					ctx.ui.setEditorText(buildTaskEditorText(task, agentName, notes, { workLoop }));
					ctx.ui.notify(`${action} applied to #${task.id}`, "info");
				} else {
					ctx.ui.setEditorText(`tak ${action} ${taskId}: ok`);
					ctx.ui.notify(`${action} applied to #${taskId}`, "info");
				}
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "graph") {
				const action = parsed.graphAction;
				if (!action) {
					ctx.ui.notify("Invalid graph action", "warning");
					return;
				}

				const taskId = parsed.graphTaskId;
				if (!taskId) {
					ctx.ui.notify(`Usage: /tak ${action} <task-id>`, "warning");
					return;
				}

				let graphArgs: string[];
				if (action === "depend" || action === "undepend") {
					const onIds = parsed.graphOnTaskIds ?? [];
					if (onIds.length === 0) {
						ctx.ui.notify(`Usage: /tak ${action} <task-id> on:<dep-id[,dep-id]>`, "warning");
						return;
					}
					graphArgs = [action, taskId, "--on", onIds.join(",")];
				} else if (action === "reparent") {
					const toId = parsed.graphToTaskId;
					if (!toId) {
						ctx.ui.notify("Usage: /tak reparent <task-id> to:<parent-id>", "warning");
						return;
					}
					graphArgs = ["reparent", taskId, "--to", toId];
				} else {
					graphArgs = ["orphan", taskId];
				}

				const graphResult = await runTak(pi, graphArgs);
				if (!graphResult.ok) {
					ctx.ui.notify(graphResult.errorMessage ?? `tak ${action} failed`, "error");
					return;
				}

				const task = coerceTakTask(graphResult.parsed);
				if (task) {
					const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", task.id]);
					const notes = coerceBlackboardNoteArray(notesResult.parsed);
					ctx.ui.setEditorText(buildTaskEditorText(task, agentName, notes, { workLoop }));
					ctx.ui.notify(`${action} applied to #${task.id}`, "info");
				} else {
					ctx.ui.setEditorText(`tak ${action} ${taskId}: ok`);
					ctx.ui.notify(`${action} applied to #${taskId}`, "info");
				}
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

			if (parsed.mode === "blackboard") {
				const action = parsed.blackboardAction;
				if (!action) {
					ctx.ui.notify("Invalid /tak blackboard action", "warning");
					return;
				}

				if (action === "post") {
					const author = parsed.blackboardBy ?? agentName;
					if (!author) {
						ctx.ui.notify("/tak blackboard post requires an author (set mesh identity or by:<name>)", "warning");
						return;
					}

					const message = parsed.blackboardMessage;
					if (!message) {
						ctx.ui.notify("Usage: /tak blackboard post message:<text>", "warning");
						return;
					}

					const postArgs = ["blackboard", "post", "--from", author, "--message", message];
					if (parsed.blackboardTemplate) {
						postArgs.push("--template", parsed.blackboardTemplate);
					}
					if (parsed.blackboardSinceNote !== undefined) {
						postArgs.push("--since-note", String(parsed.blackboardSinceNote));
					}
					if (parsed.blackboardNoChangeSince) {
						postArgs.push("--no-change-since");
					}
					for (const tag of parsed.blackboardTags ?? []) {
						postArgs.push("--tag", tag);
					}
					for (const taskId of parsed.blackboardTaskIds ?? []) {
						postArgs.push("--task", taskId);
					}

					const postResult = await runTak(pi, postArgs);
					if (!postResult.ok) {
						ctx.ui.notify(postResult.errorMessage ?? "tak blackboard post failed", "error");
						return;
					}

					const note = coerceBlackboardNote(postResult.parsed);
					if (note) {
						ctx.ui.setEditorText(`[B${note.id}] ${note.message}`);
						ctx.ui.notify(`Posted blackboard note B${note.id}`, "info");
					} else {
						ctx.ui.notify("Posted blackboard note", "info");
					}
					await refreshStatus(ctx);
					return;
				}

				if (action === "show") {
					if (parsed.blackboardId === undefined) {
						ctx.ui.notify("Usage: /tak blackboard show <note-id>", "warning");
						return;
					}

					const showResult = await runTak(pi, ["blackboard", "show", String(parsed.blackboardId)]);
					if (!showResult.ok) {
						ctx.ui.notify(showResult.errorMessage ?? "tak blackboard show failed", "error");
						return;
					}

					const note = coerceBlackboardNote(showResult.parsed);
					if (!note) {
						ctx.ui.notify("Unexpected blackboard note payload", "warning");
						return;
					}

					ctx.ui.setEditorText(`[B${note.id}] (${note.status}) ${note.author}\n\n${note.message}`);
					ctx.ui.notify(`Loaded blackboard note B${note.id}`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "close") {
					if (parsed.blackboardId === undefined) {
						ctx.ui.notify("Usage: /tak blackboard close <note-id>", "warning");
						return;
					}
					const by = parsed.blackboardBy ?? agentName;
					if (!by) {
						ctx.ui.notify("/tak blackboard close requires by:<name> or mesh identity", "warning");
						return;
					}

					const closeArgs = ["blackboard", "close", String(parsed.blackboardId), "--by", by];
					if (parsed.blackboardReason) {
						closeArgs.push("--reason", parsed.blackboardReason);
					}

					const closeResult = await runTak(pi, closeArgs);
					if (!closeResult.ok) {
						ctx.ui.notify(closeResult.errorMessage ?? "tak blackboard close failed", "error");
						return;
					}

					ctx.ui.notify(`Closed blackboard note B${parsed.blackboardId}`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "reopen") {
					if (parsed.blackboardId === undefined) {
						ctx.ui.notify("Usage: /tak blackboard reopen <note-id>", "warning");
						return;
					}
					const by = parsed.blackboardBy ?? agentName;
					if (!by) {
						ctx.ui.notify("/tak blackboard reopen requires by:<name> or mesh identity", "warning");
						return;
					}

					const reopenResult = await runTak(pi, [
						"blackboard",
						"reopen",
						String(parsed.blackboardId),
						"--by",
						by,
					]);
					if (!reopenResult.ok) {
						ctx.ui.notify(reopenResult.errorMessage ?? "tak blackboard reopen failed", "error");
						return;
					}

					ctx.ui.notify(`Reopened blackboard note B${parsed.blackboardId}`, "info");
					await refreshStatus(ctx);
					return;
				}
			}

			if (parsed.mode === "show") {
				if (!parsed.taskId) {
					ctx.ui.notify("Task id missing", "error");
					return;
				}
				const showResult = await runTak(pi, ["show", parsed.taskId]);
				const task = showResult.ok ? coerceTakTask(showResult.parsed) : null;
				if (!task) {
					ctx.ui.notify(showResult.errorMessage ?? `Could not load task ${parsed.taskId}`, "error");
					return;
				}
				const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open", "--task", task.id]);
				const notes = coerceBlackboardNoteArray(notesResult.parsed);
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
				if (!claimResult.ok) {
					ctx.ui.notify(claimResult.errorMessage ?? "No task claimed", "warning");
					await refreshStatus(ctx);
					return;
				}

				const task = coerceTakTask(claimResult.parsed);
				if (!task) {
					ctx.ui.notify("Claim returned an unexpected payload", "warning");
					await refreshStatus(ctx);
					return;
				}
				ctx.ui.setEditorText(buildTaskEditorText(task, agentName, undefined, { workLoop }));
				ctx.ui.notify(`Claimed task #${task.id}: ${task.title}`, "info");
				await refreshStatus(ctx);
				return;
			}

			if (parsed.mode === "mesh") {
				const action = parsed.meshAction ?? "summary";
				const meshPaths = parsed.meshPaths ?? [];

				if (action === "send") {
					if (!agentName) {
						ctx.ui.notify("/tak mesh send requires an agent identity", "warning");
						return;
					}
					if (!parsed.meshTo || !parsed.meshMessage) {
						ctx.ui.notify("Usage: /tak mesh send to:<agent> message:<text>", "warning");
						return;
					}

					const sendResult = await runTak(pi, [
						"mesh",
						"send",
						"--from",
						agentName,
						"--to",
						parsed.meshTo,
						"--message",
						parsed.meshMessage,
					]);
					if (!sendResult.ok) {
						ctx.ui.notify(sendResult.errorMessage ?? "tak mesh send failed", "error");
						return;
					}

					ctx.ui.notify(`Sent mesh message to ${parsed.meshTo}`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "broadcast") {
					if (!agentName) {
						ctx.ui.notify("/tak mesh broadcast requires an agent identity", "warning");
						return;
					}
					if (!parsed.meshMessage) {
						ctx.ui.notify("Usage: /tak mesh broadcast message:<text>", "warning");
						return;
					}

					const broadcastResult = await runTak(pi, [
						"mesh",
						"broadcast",
						"--from",
						agentName,
						"--message",
						parsed.meshMessage,
					]);
					if (!broadcastResult.ok) {
						ctx.ui.notify(broadcastResult.errorMessage ?? "tak mesh broadcast failed", "error");
						return;
					}

					const recipients = Array.isArray(broadcastResult.parsed)
						? (broadcastResult.parsed as unknown[]).length
						: 0;
					ctx.ui.notify(`Broadcast sent to ${recipients} agent(s)`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "reserve") {
					if (!agentName) {
						ctx.ui.notify("/tak mesh reserve requires an agent identity", "warning");
						return;
					}
					if (meshPaths.length === 0) {
						ctx.ui.notify("Usage: /tak mesh reserve path:<path> [path:<path>] [reason:<text>]", "warning");
						return;
					}

					const reserveArgs = ["mesh", "reserve", "--name", agentName];
					for (const path of meshPaths) {
						reserveArgs.push("--path", path);
					}
					if (parsed.meshReason) {
						reserveArgs.push("--reason", parsed.meshReason);
					}

					const reserveResult = await runTak(pi, reserveArgs);
					if (!reserveResult.ok) {
						ctx.ui.notify(reserveResult.errorMessage ?? "tak mesh reserve failed", "error");
						return;
					}

					ctx.ui.notify(`Reserved ${meshPaths.length} path(s)`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "release") {
					if (!agentName) {
						ctx.ui.notify("/tak mesh release requires an agent identity", "warning");
						return;
					}

					const releaseArgs = ["mesh", "release", "--name", agentName];
					if (parsed.meshAll || meshPaths.length === 0) {
						releaseArgs.push("--all");
					} else {
						for (const path of meshPaths) {
							releaseArgs.push("--path", path);
						}
					}

					const releaseResult = await runTak(pi, releaseArgs);
					if (!releaseResult.ok) {
						ctx.ui.notify(releaseResult.errorMessage ?? "tak mesh release failed", "error");
						return;
					}

					ctx.ui.notify(parsed.meshAll || meshPaths.length === 0 ? "Released all reservations" : `Released ${meshPaths.length} path(s)`, "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "feed") {
					const feedArgs = ["mesh", "feed"];
					if (parsed.filters.limit) {
						feedArgs.push("--limit", String(parsed.filters.limit));
					}

					const feedResult = await runTak(pi, feedArgs);
					if (!feedResult.ok) {
						ctx.ui.notify(feedResult.errorMessage ?? "tak mesh feed failed", "error");
						return;
					}

					const lines: string[] = ["# tak mesh feed"];
					const events = Array.isArray(feedResult.parsed) ? (feedResult.parsed as unknown[]) : [];
					if (events.length === 0) {
						lines.push("(empty)");
					} else {
						for (const rawEvent of events) {
							if (!isRecord(rawEvent)) continue;
							const ts = typeof rawEvent.ts === "string" ? rawEvent.ts : "?";
							const agent = typeof rawEvent.agent === "string" ? rawEvent.agent : "unknown";
							const type = typeof rawEvent.type === "string" ? rawEvent.type : "event";
							const target = typeof rawEvent.target === "string" ? ` -> ${rawEvent.target}` : "";
							const preview = typeof rawEvent.preview === "string" ? ` :: ${rawEvent.preview}` : "";
							lines.push(`- ${ts} ${agent} ${type}${target}${preview}`);
						}
					}

					ctx.ui.setEditorText(lines.join("\n"));
					ctx.ui.notify("Inserted /tak mesh feed", "info");
					await refreshStatus(ctx);
					return;
				}

				if (action === "blockers") {
					const blockersArgs = ["mesh", "blockers"];
					for (const path of meshPaths) {
						blockersArgs.push("--path", path);
					}

					const blockersResult = await runTak(pi, blockersArgs);
					if (!blockersResult.ok) {
						ctx.ui.notify(blockersResult.errorMessage ?? "tak mesh blockers failed", "error");
						return;
					}

					const lines: string[] = ["# tak mesh blockers"];
					if (meshPaths.length > 0) {
						lines.push(`paths: ${meshPaths.join(", ")}`);
						lines.push("");
					}

					const blockers = Array.isArray(blockersResult.parsed) ? (blockersResult.parsed as unknown[]) : [];
					if (blockers.length === 0) {
						lines.push("No active blockers.");
					} else {
						for (const rawBlocker of blockers) {
							if (!isRecord(rawBlocker)) continue;
							const owner = typeof rawBlocker.owner === "string" ? rawBlocker.owner : "unknown";
							const path = typeof rawBlocker.path === "string" ? rawBlocker.path : "?";
							const age = rawBlocker.age_secs !== undefined ? String(rawBlocker.age_secs) : "?";
							const reason = typeof rawBlocker.reason === "string" && rawBlocker.reason ? rawBlocker.reason : "-";
							lines.push(`- ${owner} blocks ${path} (age=${age}s, reason=${reason})`);
						}
					}

					ctx.ui.setEditorText(lines.join("\n"));
					ctx.ui.notify("Inserted /tak mesh blockers", "info");
					await refreshStatus(ctx);
					return;
				}

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
				const notes = coerceBlackboardNoteArray(notesResult.parsed);
				lines.push("");
				lines.push(`open blackboard notes (${notes.length}):`);
				for (const note of notes.slice(0, 5)) {
					lines.push(`- [B${note.id}] ${note.message}`);
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
				if (!notesResult.ok) {
					ctx.ui.notify(notesResult.errorMessage ?? "Could not load blackboard notes", "error");
					return;
				}

				const notes = coerceBlackboardNoteArray(notesResult.parsed);
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
					const showResult = await runTak(pi, ["show", linkedTask]);
					const linked = showResult.ok ? coerceTakTask(showResult.parsed) : null;
					if (linked) {
						ctx.ui.setEditorText(buildTaskEditorText(linked, agentName, [note], { workLoop }));
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
			if (!listResult.ok) {
				ctx.ui.notify(listResult.errorMessage ?? "Could not load tasks", "error");
				return;
			}

			let tasks = sortTasksUrgentThenOldest(coerceTakTaskArray(listResult.parsed));
			if (parsed.filters.limit && tasks.length > parsed.filters.limit) {
				tasks = tasks.slice(0, parsed.filters.limit);
			}

			if (tasks.length === 0) {
				ctx.ui.notify("No tasks for current source/filters", "info");
				await refreshStatus(ctx);
				return;
			}

			const notesResult = await runTak(pi, ["blackboard", "list", "--status", "open"]);
			const notes = coerceBlackboardNoteArray(notesResult.parsed);
			const noteCountByTask = new Map<string, number>();
			for (const note of notes) {
				for (const taskId of note.task_ids ?? []) {
					noteCountByTask.set(taskId, (noteCountByTask.get(taskId) ?? 0) + 1);
				}
			}

			const selectedId = await pickTaskFromList(
				ctx,
				`/tak ${parsed.filters.source} (urgent → oldest)`,
				tasks,
				noteCountByTask,
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
		lastHeartbeatAt = 0;
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
		await runTak(pi, ["mesh", "cleanup", "--stale"], { json: false, timeoutMs: 20000 });

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
			if (agentName) {
				lastHeartbeatAt = Date.now();
				ctx.ui.notify(`tak mesh joined as ${agentName}`, "info");

				const runtimeStatus = await runTakWorkAction(ctx, "status");
				if (runtimeStatus) {
					await handleTakWorkResponse(ctx, runtimeStatus);
				}
			}
		}

		await refreshStatus(ctx);
	});

	pi.on("session_shutdown", async () => {
		if (!integrationEnabled() || !meshJoined || !agentName) return;
		await runTak(pi, ["mesh", "leave", "--name", agentName]);
	});

	pi.on("turn_end", async (_event, ctx) => {
		await syncWorkLoop(ctx);
		await maybeHeartbeat(ctx);
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

		const reservations = filterReservationsToActiveAgents(
			loadReservations(ctx.cwd),
			loadActiveAgentNames(ctx.cwd),
		);

		if (isToolCallEventType("bash", event) && workLoop.active && workLoop.verifyMode === "isolated") {
			const command = event.input.command ?? "";
			if (isLikelyBuildOrTestCommand(command)) {
				const foreignReservations = reservationsForeignTo(agentName, reservations);
				if (foreignReservations.length > 0) {
					const verifyScopePaths = deriveVerifyScopePaths(agentName, reservations);
					if (verifyScopePaths.length === 0) {
						return {
							block: true,
							reason: formatVerifyGuardReason(agentName, verifyScopePaths, foreignReservations, []),
						};
					}

					const blockers = findVerifyOverlapBlockers(verifyScopePaths, foreignReservations);
					if (blockers.length > 0) {
						return {
							block: true,
							reason: formatVerifyGuardReason(agentName, verifyScopePaths, foreignReservations, blockers),
						};
					}
				}
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
