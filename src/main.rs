use chrono::NaiveDate;
use clap::{Parser, Subcommand, ValueEnum};
use tak::commands::blackboard::BlackboardTemplate;
use tak::metrics::{CompletionMetric, MetricsBucket};
use tak::model::{DepType, Estimate, Kind, LearningCategory, Priority, Risk, Status};
use tak::output::Format;
use tak::store::blackboard::BlackboardStatus;
use tak::store::work::{WorkClaimStrategy, WorkCoordinationVerbosity, WorkStore, WorkVerifyMode};

#[derive(Parser)]
#[command(
    name = "tak",
    version,
    about = "Git-native task manager for agentic workflows"
)]
struct Cli {
    /// Output format
    #[arg(long, global = true, value_enum, default_value = "json")]
    format: Format,
    /// Shorthand for --format pretty
    #[arg(long, global = true, hide = true)]
    pretty: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Shared coordination blackboard
    Blackboard {
        #[command(subcommand)]
        action: BlackboardAction,
    },
    /// Set a task to cancelled
    Cancel {
        /// Task ID to cancel
        id: String,
        /// Reason for cancellation (recorded as last_error)
        #[arg(long)]
        reason: Option<String>,
    },
    /// Atomically find and start the next available task
    Claim {
        /// Who is claiming the task (default: $TAK_AGENT, then auto-generated)
        #[arg(long)]
        assignee: Option<String>,
        /// Only claim tasks with this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Read or write context notes for a task
    Context {
        /// Task ID
        id: String,
        /// Set context text (overwrites existing)
        #[arg(long)]
        set: Option<String>,
        /// Clear context notes
        #[arg(long, conflicts_with = "set")]
        clear: bool,
    },
    /// Create a new task
    Create {
        /// Task title
        title: String,
        /// Task kind
        #[arg(long, value_enum, default_value = "task")]
        kind: Kind,
        /// Parent task ID (creates a child relationship)
        #[arg(long)]
        parent: Option<String>,
        /// Task IDs this task depends on (comma-separated)
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<String>,
        /// Task description
        #[arg(long, short)]
        description: Option<String>,
        /// Tags to attach (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tag: Vec<String>,
        /// One-sentence objective
        #[arg(long)]
        objective: Option<String>,
        /// Verification command (repeatable)
        #[arg(long = "verify")]
        verify: Vec<String>,
        /// Constraint the implementer must respect (repeatable)
        #[arg(long)]
        constraint: Vec<String>,
        /// Acceptance criterion (repeatable)
        #[arg(long = "criterion")]
        criterion: Vec<String>,
        /// Task priority
        #[arg(long, value_enum)]
        priority: Option<Priority>,
        /// Size estimate
        #[arg(long, value_enum)]
        estimate: Option<Estimate>,
        /// Required skill (repeatable)
        #[arg(long = "skill")]
        skill: Vec<String>,
        /// Risk level
        #[arg(long, value_enum)]
        risk: Option<Risk>,
    },
    /// Delete a task by ID
    Delete {
        /// Task ID to delete
        id: String,
        /// Cascade: orphan children and remove incoming dependencies
        #[arg(long)]
        force: bool,
    },
    /// Add dependency edges (task cannot start until deps are done)
    Depend {
        /// Task ID that will gain dependencies
        id: String,
        /// IDs of tasks it depends on (comma-separated)
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<String>,
        /// Dependency type (hard or soft)
        #[arg(long, value_enum)]
        dep_type: Option<DepType>,
        /// Reason for the dependency
        #[arg(long)]
        reason: Option<String>,
    },
    /// Validate tak installation and report issues
    Doctor {
        /// Auto-fix what can be fixed (reindex if stale, etc.)
        #[arg(long)]
        fix: bool,
    },
    /// Edit task fields
    Edit {
        /// Task ID to edit
        id: String,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New description
        #[arg(long, short)]
        description: Option<String>,
        /// New kind
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Replace tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tag: Option<Vec<String>>,
        /// Set objective
        #[arg(long)]
        objective: Option<String>,
        /// Replace verification commands (repeatable)
        #[arg(long = "verify")]
        verify: Option<Vec<String>>,
        /// Replace constraints (repeatable)
        #[arg(long)]
        constraint: Option<Vec<String>>,
        /// Replace acceptance criteria (repeatable)
        #[arg(long = "criterion")]
        criterion: Option<Vec<String>>,
        /// Set priority
        #[arg(long, value_enum)]
        priority: Option<Priority>,
        /// Set size estimate
        #[arg(long, value_enum)]
        estimate: Option<Estimate>,
        /// Replace required skills (repeatable)
        #[arg(long = "skill")]
        skill: Option<Vec<String>>,
        /// Set risk level
        #[arg(long, value_enum)]
        risk: Option<Risk>,
        /// Set pull request URL
        #[arg(long)]
        pr: Option<String>,
    },
    /// Set a task to done
    Finish {
        /// Task ID to finish
        id: String,
    },
    /// Hand off an in-progress task back to pending for another agent
    Handoff {
        /// Task ID to hand off
        id: String,
        /// Summary of progress so far (required)
        #[arg(long, required = true)]
        summary: String,
        /// Coordination verbosity override for this handoff
        #[arg(long, value_enum)]
        verbosity: Option<WorkCoordinationVerbosity>,
    },
    /// Initialize a new .tak/ directory in the current repository
    Init,
    /// Manage learnings (add, list, show, edit, remove, suggest)
    Learn {
        #[command(subcommand)]
        action: LearnAction,
    },
    /// List and filter tasks
    List {
        /// Filter by status
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// Filter by kind
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by assignee
        #[arg(long)]
        assignee: Option<String>,
        /// Show only available tasks (pending, unblocked, unassigned)
        #[arg(long, conflicts_with = "blocked")]
        available: bool,
        /// Show only blocked tasks
        #[arg(long, conflicts_with = "available")]
        blocked: bool,
        /// Show only children of this task ID
        #[arg(long)]
        children_of: Option<String>,
        /// Filter by priority
        #[arg(long, value_enum)]
        priority: Option<Priority>,
    },
    /// Display history log for a task
    Log {
        /// Task ID
        id: String,
    },
    /// Multi-agent coordination mesh
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
    /// Metrics and trend reporting
    Metrics {
        #[command(subcommand)]
        action: MetricsAction,
    },
    /// Migrate task IDs (legacy numeric filename migration and optional random re-key)
    MigrateIds {
        /// Preview migration preflight (default when --apply is not provided)
        #[arg(long, conflicts_with = "apply")]
        dry_run: bool,
        /// Apply migration changes
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,
        /// Re-key all task IDs to fresh random IDs (works on already-canonical repos)
        #[arg(long)]
        rekey_random: bool,
        /// Skip in-progress task safety gate
        #[arg(long)]
        force: bool,
    },
    /// Show the next available task without claiming it
    Next {
        /// Include tasks assigned to this person
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Remove a task's parent (make it a root task)
    Orphan {
        /// Task ID to orphan
        id: String,
    },
    /// Rebuild the SQLite index from task files
    Reindex,
    /// Reopen a done or cancelled task back to pending
    Reopen {
        /// Task ID to reopen
        id: String,
    },
    /// Change a task's parent
    Reparent {
        /// Task ID to reparent
        id: String,
        /// New parent task ID
        #[arg(long, required = true)]
        to: String,
    },
    /// Install agent integrations (Claude hooks; optional Claude plugin/skills and pi integration)
    Setup {
        /// Write to ~/.claude/settings.json instead of .claude/settings.local.json
        #[arg(long)]
        global: bool,
        /// Verify installation status, exit 0/1
        #[arg(long)]
        check: bool,
        /// Remove tak hooks from settings
        #[arg(long)]
        remove: bool,
        /// Also write the full Claude plugin directory to .claude/plugins/tak
        #[arg(long)]
        plugin: bool,
        /// Install Claude skills under .claude/skills/ (use alone for skills-only setup)
        #[arg(long)]
        skills: bool,
        /// Also install pi integration (extension, skill, and APPEND_SYSTEM block)
        #[arg(long)]
        pi: bool,
    },
    /// Display a single task
    Show {
        /// Task ID to show
        id: String,
    },
    /// Set a task to in_progress
    Start {
        /// Task ID to start
        id: String,
        /// Who is working on it
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Diagnose /tak workflow friction and record therapist observations
    Therapist {
        #[command(subcommand)]
        action: TherapistAction,
    },
    /// Display the parent-child task hierarchy
    Tree {
        /// Root task ID (omit for full tree)
        #[arg(value_name = "ID")]
        root_id: Option<String>,
        /// Root task ID (named form)
        #[arg(long = "id", value_name = "ID", conflicts_with = "root_id")]
        id: Option<String>,
        /// Show only pending tasks
        #[arg(long)]
        pending: bool,
    },
    /// Clear a task's assignee without changing status
    Unassign {
        /// Task ID to unassign
        id: String,
    },
    /// Remove dependency edges
    Undepend {
        /// Task ID to remove dependencies from
        id: String,
        /// IDs of dependencies to remove (comma-separated)
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<String>,
    },
    /// Run verification commands from task contract
    Verify {
        /// Task ID
        id: String,
    },
    /// Deterministically wait for a path reservation to clear or a task to become unblocked
    Wait {
        /// Wait until this path is no longer blocked by a conflicting reservation
        #[arg(long, conflicts_with = "on_task", required_unless_present = "on_task")]
        path: Option<String>,
        /// Wait until this task is no longer blocked by unfinished dependencies
        #[arg(
            long = "on-task",
            conflicts_with = "path",
            required_unless_present = "path"
        )]
        on_task: Option<String>,
        /// Timeout in seconds (omit to wait indefinitely)
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Manage CLI-native work-loop runtime state (`tak work`, `tak work status`, `tak work stop`, `tak work done`)
    Work {
        /// Optional action (default: start/resume)
        #[arg(value_enum)]
        action: Option<WorkAction>,
        /// Agent identity override (`--assignee` > `TAK_AGENT` > generated fallback)
        #[arg(long)]
        assignee: Option<String>,
        /// Optional tag filter persisted in loop state
        #[arg(long)]
        tag: Option<String>,
        /// Optional loop limit (remaining units)
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        limit: Option<u32>,
        /// Verification mode policy hint to persist in loop state
        #[arg(long, value_enum)]
        verify: Option<WorkVerifyMode>,
        /// Claim prioritization strategy for selecting the next task
        #[arg(long, value_enum)]
        strategy: Option<WorkClaimStrategy>,
        /// Default coordination verbosity to persist in loop state
        #[arg(long, value_enum)]
        verbosity: Option<WorkCoordinationVerbosity>,
        /// Bypass safe-resume anti-thrash gate and allow immediate reclaim attempt
        #[arg(long)]
        force_reclaim: bool,
        /// Pause/deactivate the loop after `tak work done`
        #[arg(long)]
        pause: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "snake_case")]
enum WorkAction {
    Start,
    Status,
    Stop,
    Done,
}

#[derive(Subcommand)]
enum LearnAction {
    /// Record a new learning
    Add {
        /// Learning title
        title: String,
        /// Detailed description
        #[arg(long, short)]
        description: Option<String>,
        /// Category
        #[arg(long, value_enum, default_value = "insight")]
        category: LearningCategory,
        /// Tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tag: Vec<String>,
        /// Link to task IDs (comma-separated)
        #[arg(long = "task", value_delimiter = ',')]
        task_ids: Vec<String>,
    },
    /// List learnings with optional filters
    List {
        /// Filter by category
        #[arg(long, value_enum)]
        category: Option<LearningCategory>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by linked task ID
        #[arg(long = "task")]
        task_id: Option<String>,
    },
    /// Display a single learning
    Show {
        /// Learning ID
        id: u64,
    },
    /// Edit learning fields
    Edit {
        /// Learning ID
        id: u64,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New description
        #[arg(long, short)]
        description: Option<String>,
        /// New category
        #[arg(long, value_enum)]
        category: Option<LearningCategory>,
        /// Replace tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tag: Option<Vec<String>>,
        /// Add link to task ID (repeatable)
        #[arg(long = "add-task", value_delimiter = ',')]
        add_task: Vec<String>,
        /// Remove link to task ID (repeatable)
        #[arg(long = "remove-task", value_delimiter = ',')]
        remove_task: Vec<String>,
    },
    /// Remove a learning
    Remove {
        /// Learning ID
        id: u64,
    },
    /// Suggest relevant learnings for a task (FTS5 search)
    Suggest {
        /// Task ID to suggest learnings for
        task_id: String,
    },
}

#[derive(Subcommand)]
enum MetricsAction {
    /// Burndown trend over a date window
    Burndown {
        /// Start date (inclusive, YYYY-MM-DD). Defaults to 30 days before --to.
        #[arg(long)]
        from: Option<NaiveDate>,
        /// End date (inclusive, YYYY-MM-DD). Defaults to today.
        #[arg(long)]
        to: Option<NaiveDate>,
        /// Aggregation bucket
        #[arg(long, value_enum, default_value = "day")]
        bucket: MetricsBucket,
        /// Filter by task kind
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Require tags (repeatable or comma-separated)
        #[arg(long = "tag", value_delimiter = ',')]
        tag: Vec<String>,
        /// Filter by assignee
        #[arg(long)]
        assignee: Option<String>,
        /// Filter by parent task ID (children only)
        #[arg(long)]
        children_of: Option<String>,
        /// Include cancelled tasks in source set
        #[arg(long)]
        include_cancelled: bool,
    },
    /// Completion-time trend over a date window
    CompletionTime {
        /// Start date (inclusive, YYYY-MM-DD). Defaults to 30 days before --to.
        #[arg(long)]
        from: Option<NaiveDate>,
        /// End date (inclusive, YYYY-MM-DD). Defaults to today.
        #[arg(long)]
        to: Option<NaiveDate>,
        /// Aggregation bucket
        #[arg(long, value_enum, default_value = "day")]
        bucket: MetricsBucket,
        /// Duration metric (lead or cycle)
        #[arg(long, value_enum, default_value = "cycle")]
        metric: CompletionMetric,
        /// Filter by task kind
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// Require tags (repeatable or comma-separated)
        #[arg(long = "tag", value_delimiter = ',')]
        tag: Vec<String>,
        /// Filter by assignee
        #[arg(long)]
        assignee: Option<String>,
        /// Filter by parent task ID (children only)
        #[arg(long)]
        children_of: Option<String>,
        /// Include cancelled tasks in source set
        #[arg(long)]
        include_cancelled: bool,
    },
}

#[derive(Subcommand)]
enum MeshAction {
    /// Register this agent in the mesh
    Join {
        /// Agent name (auto-generated if omitted)
        #[arg(long)]
        name: Option<String>,
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Unregister from the mesh
    Leave {
        /// Agent name (optional; resolves from current session when omitted)
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered agents
    List,
    /// Send a direct message to an agent
    Send {
        /// Sender name
        #[arg(long)]
        from: String,
        /// Recipient name
        #[arg(long)]
        to: String,
        /// Message text
        #[arg(long)]
        message: String,
        /// Coordination verbosity override for this message
        #[arg(long, value_enum)]
        verbosity: Option<WorkCoordinationVerbosity>,
    },
    /// Broadcast a message to all agents
    Broadcast {
        /// Sender name
        #[arg(long)]
        from: String,
        /// Message text
        #[arg(long)]
        message: String,
        /// Coordination verbosity override for this broadcast
        #[arg(long, value_enum)]
        verbosity: Option<WorkCoordinationVerbosity>,
    },
    /// Read inbox messages
    Inbox {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Acknowledge (delete) all messages after reading
        #[arg(long, conflicts_with_all = ["ack_ids", "ack_before"])]
        ack: bool,
        /// Acknowledge specific message IDs (repeatable)
        #[arg(long = "ack-id", value_name = "ID", conflicts_with = "ack")]
        ack_ids: Vec<String>,
        /// Acknowledge all messages up to and including this message ID
        #[arg(long = "ack-before", value_name = "ID", conflicts_with = "ack")]
        ack_before: Option<String>,
    },
    /// Refresh registration and reservation lease liveness metadata
    Heartbeat {
        /// Agent name (optional; resolves from session/cwd when omitted)
        #[arg(long)]
        name: Option<String>,
        /// Session ID override for implicit resolution
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Clean up stale mesh runtime state
    Cleanup {
        /// Remove stale registrations/reservations
        #[arg(long, required = true)]
        stale: bool,
        /// Preview what would be removed without mutating state
        #[arg(long)]
        dry_run: bool,
        /// Override stale detection TTL (seconds)
        #[arg(long)]
        ttl_seconds: Option<u64>,
    },
    /// Diagnose active reservation blockers (owner/path/reason/age)
    Blockers {
        /// Optional path filters (repeatable). When omitted, shows all active blockers.
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// Reserve file paths for exclusive editing
    Reserve {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Paths to reserve (repeatable)
        #[arg(long = "path", required = true)]
        paths: Vec<String>,
        /// Reason for reservation
        #[arg(long)]
        reason: Option<String>,
    },
    /// Release file path reservations
    Release {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Specific paths to release (omit for all)
        #[arg(long = "path")]
        paths: Vec<String>,
        /// Release all reservations for this agent
        #[arg(long, conflicts_with = "paths")]
        all: bool,
    },
    /// Show the activity feed
    Feed {
        /// Show only the last N events
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Subcommand)]
enum BlackboardAction {
    /// Post a shared note to the blackboard
    Post {
        /// Author/agent name
        #[arg(long)]
        from: String,
        /// Message text (summary when --template is used)
        #[arg(long)]
        message: String,
        /// Optional structured template for high-signal coordination notes
        #[arg(long, value_enum)]
        template: Option<BlackboardTemplate>,
        /// Reference a previous note for compact delta updates
        #[arg(long = "since-note")]
        since_note: Option<u64>,
        /// Mark this note as unchanged since --since-note
        #[arg(long = "no-change-since", requires = "since_note")]
        no_change_since: bool,
        /// Coordination verbosity override for this note
        #[arg(long, value_enum)]
        verbosity: Option<WorkCoordinationVerbosity>,
        /// Tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tag: Vec<String>,
        /// Link note to task IDs (comma-separated)
        #[arg(long = "task", value_delimiter = ',')]
        task_ids: Vec<String>,
    },
    /// List blackboard notes with optional filters
    List {
        /// Filter by status
        #[arg(long, value_enum)]
        status: Option<BlackboardStatus>,
        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,
        /// Filter by linked task ID
        #[arg(long = "task")]
        task_id: Option<String>,
        /// Show only the most recent N notes
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show one note
    Show {
        /// Blackboard note ID
        id: u64,
    },
    /// Close (resolve) a note
    Close {
        /// Blackboard note ID
        id: u64,
        /// Agent closing the note
        #[arg(long)]
        by: String,
        /// Optional closure reason
        #[arg(long)]
        reason: Option<String>,
    },
    /// Re-open a closed note
    Reopen {
        /// Blackboard note ID
        id: u64,
        /// Agent re-opening the note
        #[arg(long)]
        by: String,
    },
}

#[derive(Subcommand)]
enum TherapistAction {
    /// Diagnose conflict/churn from mesh history + blackboard and append an observation
    Offline {
        /// Agent/requester identity recorded in the observation
        #[arg(long)]
        by: Option<String>,
        /// Number of recent feed/blackboard records to inspect
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Resume a pi session in RPC mode and run a targeted workflow interview
    Online {
        /// Session identifier or path to resume (defaults to latest session with /tak work markers)
        #[arg(long)]
        session: Option<String>,
        /// Session directory root (default: ~/.pi/agent/sessions)
        #[arg(long)]
        session_dir: Option<String>,
        /// Agent/requester identity recorded in the observation
        #[arg(long)]
        by: Option<String>,
    },
    /// Read therapist observations from the append-only log
    Log {
        /// Show only the most recent N observations
        #[arg(long)]
        limit: Option<usize>,
    },
}

fn resolve_task_id_arg(repo_root: &std::path::Path, input: String) -> tak::error::Result<u64> {
    let repo = tak::store::repo::Repo::open(repo_root)?;
    repo.resolve_task_id_u64(&input)
}

fn resolve_optional_task_id_arg(
    repo_root: &std::path::Path,
    input: Option<String>,
) -> tak::error::Result<Option<u64>> {
    input
        .map(|id| resolve_task_id_arg(repo_root, id))
        .transpose()
}

fn resolve_task_id_args(
    repo_root: &std::path::Path,
    inputs: Vec<String>,
) -> tak::error::Result<Vec<u64>> {
    inputs
        .into_iter()
        .map(|id| resolve_task_id_arg(repo_root, id))
        .collect()
}

fn resolve_effective_coordination_verbosity(
    repo_root: &std::path::Path,
    agent: Option<&str>,
    override_level: Option<WorkCoordinationVerbosity>,
) -> WorkCoordinationVerbosity {
    if let Some(level) = override_level {
        return level;
    }

    let Some(agent) = agent else {
        return WorkCoordinationVerbosity::default();
    };

    let store = WorkStore::open(&repo_root.join(".tak"));
    store
        .status(agent)
        .map(|state| state.coordination_verbosity)
        .unwrap_or_default()
}

fn apply_coordination_verbosity_label(
    message: &str,
    level: WorkCoordinationVerbosity,
    explicit_override: bool,
) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if !explicit_override && level == WorkCoordinationVerbosity::Medium {
        return trimmed.to_string();
    }

    format!("[verbosity={level}] {trimmed}")
}

fn maybe_add_verbosity_tag(
    tags: &mut Vec<String>,
    level: WorkCoordinationVerbosity,
    explicit_override: bool,
) {
    if !explicit_override && level == WorkCoordinationVerbosity::Medium {
        return;
    }

    tags.push(format!("verbosity-{level}"));
}

fn task_assignee_for_verbosity(
    repo_root: &std::path::Path,
    task_id: u64,
) -> tak::error::Result<Option<String>> {
    let repo = tak::store::repo::Repo::open(repo_root)?;
    let task = repo.store.read(task_id)?;
    Ok(task.assignee)
}

fn run(cli: Cli, format: Format) -> tak::error::Result<()> {
    // Commands dispatched before `.tak` repo discovery
    match &cli.command {
        Commands::Init => {
            let cwd = std::env::current_dir()?;
            return tak::commands::init::run(&cwd);
        }
        Commands::Setup {
            global,
            check,
            remove,
            plugin,
            skills,
            pi,
        } => {
            return tak::commands::setup::run(
                *global, *check, *remove, *plugin, *skills, *pi, format,
            );
        }
        Commands::Doctor { fix } => {
            return tak::commands::doctor::run(*fix, format);
        }
        _ => {}
    }

    let root = tak::store::repo::find_repo_root()?;

    match cli.command {
        Commands::Init | Commands::Setup { .. } | Commands::Doctor { .. } => unreachable!(),
        Commands::Create {
            title,
            kind,
            parent,
            depends_on,
            description,
            tag,
            objective,
            verify,
            constraint,
            criterion,
            priority,
            estimate,
            skill,
            risk,
        } => {
            let parent = resolve_optional_task_id_arg(&root, parent)?;
            let depends_on = resolve_task_id_args(&root, depends_on)?;
            let contract = tak::model::Contract {
                objective,
                acceptance_criteria: criterion,
                verification: verify,
                constraints: constraint,
            };
            let planning = tak::model::Planning {
                priority,
                estimate,
                required_skills: skill,
                risk,
            };
            tak::commands::create::run(
                &root,
                title,
                kind,
                description,
                parent,
                depends_on,
                tag,
                contract,
                planning,
                format,
            )
        }
        Commands::Delete { id, force } => {
            tak::commands::delete::run(&root, resolve_task_id_arg(&root, id)?, force, format)
        }
        Commands::Show { id } => {
            tak::commands::show::run(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::List {
            status,
            kind,
            tag,
            assignee,
            available,
            blocked,
            children_of,
            priority,
        } => {
            let children_of = resolve_optional_task_id_arg(&root, children_of)?;
            tak::commands::list::run(
                &root,
                status,
                kind,
                tag,
                assignee,
                available,
                blocked,
                children_of,
                priority,
                format,
            )
        }
        Commands::Edit {
            id,
            title,
            description,
            kind,
            tag,
            objective,
            verify,
            constraint,
            criterion,
            priority,
            estimate,
            skill,
            risk,
            pr,
        } => tak::commands::edit::run(
            &root,
            resolve_task_id_arg(&root, id)?,
            title,
            description,
            kind,
            tag,
            objective,
            verify,
            constraint,
            criterion,
            priority,
            estimate,
            skill,
            risk,
            pr,
            format,
        ),
        Commands::Start { id, assignee } => {
            let assignee = assignee.or_else(tak::agent::resolve_agent);
            tak::commands::lifecycle::start(
                &root,
                resolve_task_id_arg(&root, id)?,
                assignee,
                format,
            )
        }
        Commands::Finish { id } => {
            tak::commands::lifecycle::finish(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Cancel { id, reason } => {
            tak::commands::lifecycle::cancel(&root, resolve_task_id_arg(&root, id)?, reason, format)
        }
        Commands::Handoff {
            id,
            summary,
            verbosity,
        } => {
            let task_id = resolve_task_id_arg(&root, id)?;
            let assignee = task_assignee_for_verbosity(&root, task_id)?;
            let effective =
                resolve_effective_coordination_verbosity(&root, assignee.as_deref(), verbosity);
            let summary =
                apply_coordination_verbosity_label(&summary, effective, verbosity.is_some());
            tak::commands::lifecycle::handoff(&root, task_id, summary, format)
        }
        Commands::Claim { assignee, tag } => {
            let assignee = assignee
                .or_else(tak::agent::resolve_agent)
                .unwrap_or_else(tak::agent::generated_fallback);
            tak::commands::claim::run(&root, assignee, tag, format)
        }
        Commands::Reopen { id } => {
            tak::commands::lifecycle::reopen(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Unassign { id } => {
            tak::commands::lifecycle::unassign(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Depend {
            id,
            on,
            dep_type,
            reason,
        } => tak::commands::deps::depend(
            &root,
            resolve_task_id_arg(&root, id)?,
            resolve_task_id_args(&root, on)?,
            dep_type,
            reason,
            format,
        ),
        Commands::Undepend { id, on } => tak::commands::deps::undepend(
            &root,
            resolve_task_id_arg(&root, id)?,
            resolve_task_id_args(&root, on)?,
            format,
        ),
        Commands::Reparent { id, to } => tak::commands::deps::reparent(
            &root,
            resolve_task_id_arg(&root, id)?,
            resolve_task_id_arg(&root, to)?,
            format,
        ),
        Commands::Orphan { id } => {
            tak::commands::deps::orphan(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Tree {
            root_id,
            id,
            pending,
        } => tak::commands::tree::run(
            &root,
            resolve_optional_task_id_arg(&root, id.or(root_id))?,
            pending,
            format,
        ),
        Commands::Next { assignee } => tak::commands::next::run(&root, assignee, format),
        Commands::Wait {
            path,
            on_task,
            timeout,
        } => tak::commands::wait::run(
            &root,
            path,
            resolve_optional_task_id_arg(&root, on_task)?,
            timeout,
            format,
        ),
        Commands::Work {
            action,
            assignee,
            tag,
            limit,
            verify,
            strategy,
            verbosity,
            force_reclaim,
            pause,
        } => match action.unwrap_or(WorkAction::Start) {
            WorkAction::Start => tak::commands::work::start_or_resume_with_strategy_force(
                &root,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                format,
            ),
            WorkAction::Status => tak::commands::work::status(&root, assignee, format),
            WorkAction::Stop => tak::commands::work::stop(&root, assignee, format),
            WorkAction::Done => tak::commands::work::done(&root, assignee, pause, format),
        },
        Commands::Verify { id } => {
            tak::commands::verify::run(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Log { id } => {
            tak::commands::log::run(&root, resolve_task_id_arg(&root, id)?, format)
        }
        Commands::Context { id, set, clear } => {
            tak::commands::context::run(&root, resolve_task_id_arg(&root, id)?, set, clear, format)
        }
        Commands::Learn { action } => match action {
            LearnAction::Add {
                title,
                description,
                category,
                tag,
                task_ids,
            } => tak::commands::learn::add(
                &root,
                title,
                description,
                category,
                tag,
                resolve_task_id_args(&root, task_ids)?,
                format,
            ),
            LearnAction::List {
                category,
                tag,
                task_id,
            } => tak::commands::learn::list(
                &root,
                category,
                tag,
                resolve_optional_task_id_arg(&root, task_id)?,
                format,
            ),
            LearnAction::Show { id } => tak::commands::learn::show(&root, id, format),
            LearnAction::Edit {
                id,
                title,
                description,
                category,
                tag,
                add_task,
                remove_task,
            } => tak::commands::learn::edit(
                &root,
                id,
                title,
                description,
                category,
                tag,
                resolve_task_id_args(&root, add_task)?,
                resolve_task_id_args(&root, remove_task)?,
                format,
            ),
            LearnAction::Remove { id } => tak::commands::learn::remove(&root, id, format),
            LearnAction::Suggest { task_id } => {
                tak::commands::learn::suggest(&root, resolve_task_id_arg(&root, task_id)?, format)
            }
        },
        Commands::Metrics { action } => match action {
            MetricsAction::Burndown {
                from,
                to,
                bucket,
                kind,
                tag,
                assignee,
                children_of,
                include_cancelled,
            } => tak::commands::metrics::burndown(
                &root,
                from,
                to,
                bucket,
                kind,
                tag,
                assignee,
                resolve_optional_task_id_arg(&root, children_of)?,
                include_cancelled,
                format,
            ),
            MetricsAction::CompletionTime {
                from,
                to,
                bucket,
                metric,
                kind,
                tag,
                assignee,
                children_of,
                include_cancelled,
            } => tak::commands::metrics::completion_time(
                &root,
                from,
                to,
                bucket,
                kind,
                tag,
                assignee,
                resolve_optional_task_id_arg(&root, children_of)?,
                include_cancelled,
                metric,
                format,
            ),
        },
        Commands::Mesh { action } => match action {
            MeshAction::Join { name, session_id } => {
                tak::commands::mesh::join(&root, name.as_deref(), session_id.as_deref(), format)
            }
            MeshAction::Leave { name } => {
                tak::commands::mesh::leave(&root, name.as_deref(), format)
            }
            MeshAction::List => tak::commands::mesh::list(&root, format),
            MeshAction::Send {
                from,
                to,
                message,
                verbosity,
            } => {
                let effective =
                    resolve_effective_coordination_verbosity(&root, Some(&from), verbosity);
                let message =
                    apply_coordination_verbosity_label(&message, effective, verbosity.is_some());
                tak::commands::mesh::send(&root, &from, &to, &message, format)
            }
            MeshAction::Broadcast {
                from,
                message,
                verbosity,
            } => {
                let effective =
                    resolve_effective_coordination_verbosity(&root, Some(&from), verbosity);
                let message =
                    apply_coordination_verbosity_label(&message, effective, verbosity.is_some());
                tak::commands::mesh::broadcast(&root, &from, &message, format)
            }
            MeshAction::Inbox {
                name,
                ack,
                ack_ids,
                ack_before,
            } => tak::commands::mesh::inbox(
                &root,
                &name,
                ack,
                ack_ids,
                ack_before.as_deref(),
                format,
            ),
            MeshAction::Heartbeat { name, session_id } => tak::commands::mesh::heartbeat(
                &root,
                name.as_deref(),
                session_id.as_deref(),
                format,
            ),
            MeshAction::Cleanup {
                stale,
                dry_run,
                ttl_seconds,
            } => tak::commands::mesh::cleanup(&root, stale, dry_run, ttl_seconds, format),
            MeshAction::Blockers { paths } => tak::commands::mesh::blockers(&root, paths, format),
            MeshAction::Reserve {
                name,
                paths,
                reason,
            } => tak::commands::mesh::reserve(&root, &name, paths, reason.as_deref(), format),
            MeshAction::Release { name, paths, all } => {
                tak::commands::mesh::release(&root, &name, paths, all, format)
            }
            MeshAction::Feed { limit } => tak::commands::mesh::feed(&root, limit, format),
        },
        Commands::Blackboard { action } => match action {
            BlackboardAction::Post {
                from,
                message,
                template,
                since_note,
                no_change_since,
                verbosity,
                mut tag,
                task_ids,
            } => {
                let effective =
                    resolve_effective_coordination_verbosity(&root, Some(&from), verbosity);
                let message =
                    apply_coordination_verbosity_label(&message, effective, verbosity.is_some());
                maybe_add_verbosity_tag(&mut tag, effective, verbosity.is_some());

                tak::commands::blackboard::post_with_options(
                    &root,
                    &from,
                    &message,
                    tak::commands::blackboard::BlackboardPostOptions {
                        template,
                        since_note,
                        no_change_since,
                    },
                    tag,
                    resolve_task_id_args(&root, task_ids)?,
                    format,
                )
            }
            BlackboardAction::List {
                status,
                tag,
                task_id,
                limit,
            } => tak::commands::blackboard::list(
                &root,
                status,
                tag,
                resolve_optional_task_id_arg(&root, task_id)?,
                limit,
                format,
            ),
            BlackboardAction::Show { id } => tak::commands::blackboard::show(&root, id, format),
            BlackboardAction::Close { id, by, reason } => {
                tak::commands::blackboard::close(&root, id, &by, reason.as_deref(), format)
            }
            BlackboardAction::Reopen { id, by } => {
                tak::commands::blackboard::reopen(&root, id, &by, format)
            }
        },
        Commands::Therapist { action } => match action {
            TherapistAction::Offline { by, limit } => {
                tak::commands::therapist::offline(&root, by, limit, format)
            }
            TherapistAction::Online {
                session,
                session_dir,
                by,
            } => tak::commands::therapist::online(&root, session, session_dir, by, format),
            TherapistAction::Log { limit } => tak::commands::therapist::log(&root, limit, format),
        },
        Commands::MigrateIds {
            dry_run,
            apply,
            rekey_random,
            force,
        } => {
            let dry_run = dry_run || !apply;
            tak::commands::migrate_ids::run(&root, dry_run, force, rekey_random, format)
        }
        Commands::Reindex => tak::commands::reindex::run(&root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tak::error::TakError;
    use tak::model::{Contract, Kind, Planning};
    use tak::store::files::FileStore;
    use tempfile::tempdir;

    fn init_repo_with_tasks(count: usize) -> (tempfile::TempDir, Vec<u64>) {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        let mut ids = Vec::new();
        for i in 0..count {
            let task = store
                .create(
                    format!("task-{i}"),
                    Kind::Task,
                    None,
                    None,
                    vec![],
                    vec![],
                    Contract::default(),
                    Planning::default(),
                )
                .unwrap();
            ids.push(task.id);
        }
        (dir, ids)
    }

    #[test]
    fn resolve_task_id_arg_accepts_canonical_hex_input() {
        let (dir, ids) = init_repo_with_tasks(1);

        let canonical = format!("{:016x}", ids[0]);
        let resolved = resolve_task_id_arg(dir.path(), canonical).unwrap();
        assert_eq!(resolved, ids[0]);
    }

    #[test]
    fn resolve_task_id_arg_surfaces_ambiguous_prefix() {
        let (dir, ids) = init_repo_with_tasks(32);

        let mut by_prefix: HashMap<char, Vec<u64>> = HashMap::new();
        for id in ids {
            let hex = format!("{:016x}", id);
            let prefix = hex.chars().next().unwrap();
            by_prefix.entry(prefix).or_default().push(id);
        }

        let ambiguous_prefix = by_prefix
            .iter()
            .find_map(|(prefix, bucket)| (bucket.len() >= 2).then(|| prefix.to_string()))
            .expect("at least one first-hex-digit collision with 32 IDs");

        let err = resolve_task_id_arg(dir.path(), ambiguous_prefix).unwrap_err();
        assert!(matches!(err, TakError::TaskIdAmbiguous(_, _)));
    }

    #[test]
    fn apply_coordination_verbosity_label_skips_default_medium_without_override() {
        let rendered = apply_coordination_verbosity_label(
            "status update",
            WorkCoordinationVerbosity::Medium,
            false,
        );
        assert_eq!(rendered, "status update");
    }

    #[test]
    fn apply_coordination_verbosity_label_adds_marker_when_needed() {
        let rendered = apply_coordination_verbosity_label(
            "status update",
            WorkCoordinationVerbosity::High,
            false,
        );
        assert_eq!(rendered, "[verbosity=high] status update");
    }

    #[test]
    fn apply_coordination_verbosity_label_keeps_empty_input_empty() {
        let rendered =
            apply_coordination_verbosity_label("   ", WorkCoordinationVerbosity::High, true);
        assert_eq!(rendered, "");
    }

    #[test]
    fn maybe_add_verbosity_tag_skips_default_medium_without_override() {
        let mut tags = vec!["coordination".to_string()];
        maybe_add_verbosity_tag(&mut tags, WorkCoordinationVerbosity::Medium, false);
        assert_eq!(tags, vec!["coordination"]);

        maybe_add_verbosity_tag(&mut tags, WorkCoordinationVerbosity::High, false);
        assert_eq!(tags, vec!["coordination", "verbosity-high"]);
    }

    #[test]
    fn parse_create_kind_meta() {
        let cli = Cli::parse_from(["tak", "create", "Meta task", "--kind", "meta"]);

        match cli.command {
            Commands::Create { title, kind, .. } => {
                assert_eq!(title, "Meta task");
                assert_eq!(kind, Kind::Meta);
            }
            _ => panic!("expected create command"),
        }
    }

    #[test]
    fn parse_edit_kind_meta() {
        let cli = Cli::parse_from(["tak", "edit", "42", "--kind", "meta"]);

        match cli.command {
            Commands::Edit { id, kind, .. } => {
                assert_eq!(id, "42");
                assert_eq!(kind, Some(Kind::Meta));
            }
            _ => panic!("expected edit command"),
        }
    }

    #[test]
    fn parse_list_kind_meta() {
        let cli = Cli::parse_from(["tak", "list", "--kind", "meta"]);

        match cli.command {
            Commands::List { kind, .. } => {
                assert_eq!(kind, Some(Kind::Meta));
            }
            _ => panic!("expected list command"),
        }
    }

    #[test]
    fn parse_blackboard_post_template_flag() {
        let cli = Cli::parse_from([
            "tak",
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "Need help with reservation release",
            "--template",
            "blocker",
        ]);

        match cli.command {
            Commands::Blackboard {
                action: BlackboardAction::Post { from, template, .. },
            } => {
                assert_eq!(from, "agent-1");
                assert_eq!(template, Some(BlackboardTemplate::Blocker));
            }
            _ => panic!("expected blackboard post command"),
        }
    }

    #[test]
    fn parse_blackboard_post_verbosity_flag() {
        let cli = Cli::parse_from([
            "tak",
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "status update",
            "--verbosity",
            "high",
        ]);

        match cli.command {
            Commands::Blackboard {
                action: BlackboardAction::Post { verbosity, .. },
            } => {
                assert_eq!(verbosity, Some(WorkCoordinationVerbosity::High));
            }
            _ => panic!("expected blackboard post command"),
        }
    }

    #[test]
    fn parse_blackboard_post_delta_flags() {
        let cli = Cli::parse_from([
            "tak",
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "No changes from previous status",
            "--since-note",
            "42",
            "--no-change-since",
        ]);

        match cli.command {
            Commands::Blackboard {
                action:
                    BlackboardAction::Post {
                        since_note,
                        no_change_since,
                        ..
                    },
            } => {
                assert_eq!(since_note, Some(42));
                assert!(no_change_since);
            }
            _ => panic!("expected blackboard post command"),
        }
    }

    #[test]
    fn parse_blackboard_post_no_change_since_requires_since_note() {
        let result = Cli::try_parse_from([
            "tak",
            "blackboard",
            "post",
            "--from",
            "agent-1",
            "--message",
            "unchanged",
            "--no-change-since",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn parse_mesh_send_verbosity_flag() {
        let cli = Cli::parse_from([
            "tak",
            "mesh",
            "send",
            "--from",
            "agent-1",
            "--to",
            "agent-2",
            "--message",
            "ping",
            "--verbosity",
            "low",
        ]);

        match cli.command {
            Commands::Mesh {
                action:
                    MeshAction::Send {
                        from,
                        to,
                        message,
                        verbosity,
                    },
            } => {
                assert_eq!(from, "agent-1");
                assert_eq!(to, "agent-2");
                assert_eq!(message, "ping");
                assert_eq!(verbosity, Some(WorkCoordinationVerbosity::Low));
            }
            _ => panic!("expected mesh send command"),
        }
    }

    #[test]
    fn parse_mesh_inbox_selective_ack_flags() {
        let cli = Cli::parse_from([
            "tak",
            "mesh",
            "inbox",
            "--name",
            "agent-1",
            "--ack-id",
            "msg-1",
            "--ack-id",
            "msg-2",
            "--ack-before",
            "msg-9",
        ]);

        match cli.command {
            Commands::Mesh {
                action:
                    MeshAction::Inbox {
                        name,
                        ack,
                        ack_ids,
                        ack_before,
                    },
            } => {
                assert_eq!(name, "agent-1");
                assert!(!ack);
                assert_eq!(ack_ids, vec!["msg-1", "msg-2"]);
                assert_eq!(ack_before.as_deref(), Some("msg-9"));
            }
            _ => panic!("expected mesh inbox command"),
        }
    }

    #[test]
    fn parse_mesh_inbox_bulk_ack_conflicts_with_selective_flags() {
        let result = Cli::try_parse_from([
            "tak", "mesh", "inbox", "--name", "agent-1", "--ack", "--ack-id", "msg-1",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn parse_handoff_verbosity_flag() {
        let cli = Cli::parse_from([
            "tak",
            "handoff",
            "42",
            "--summary",
            "blocked by reservation",
            "--verbosity",
            "high",
        ]);

        match cli.command {
            Commands::Handoff {
                id,
                summary,
                verbosity,
            } => {
                assert_eq!(id, "42");
                assert_eq!(summary, "blocked by reservation");
                assert_eq!(verbosity, Some(WorkCoordinationVerbosity::High));
            }
            _ => panic!("expected handoff command"),
        }
    }

    #[test]
    fn parse_tree_pending_flag() {
        let cli = Cli::parse_from(["tak", "tree", "--pending"]);
        match cli.command {
            Commands::Tree {
                root_id,
                id,
                pending,
            } => {
                assert!(root_id.is_none());
                assert!(id.is_none());
                assert!(pending);
            }
            _ => panic!("expected tree command"),
        }
    }

    #[test]
    fn parse_tree_positional_id() {
        let cli = Cli::parse_from(["tak", "tree", "123"]);
        match cli.command {
            Commands::Tree {
                root_id,
                id,
                pending,
            } => {
                assert_eq!(root_id.as_deref(), Some("123"));
                assert!(id.is_none());
                assert!(!pending);
            }
            _ => panic!("expected tree command"),
        }
    }

    #[test]
    fn parse_tree_named_id() {
        let cli = Cli::parse_from(["tak", "tree", "--id", "123"]);
        match cli.command {
            Commands::Tree {
                root_id,
                id,
                pending,
            } => {
                assert!(root_id.is_none());
                assert_eq!(id.as_deref(), Some("123"));
                assert!(!pending);
            }
            _ => panic!("expected tree command"),
        }
    }

    #[test]
    fn parse_mesh_blockers_paths() {
        let cli = Cli::parse_from([
            "tak",
            "mesh",
            "blockers",
            "--path",
            "src/store/mesh.rs",
            "--path",
            "README.md",
        ]);

        match cli.command {
            Commands::Mesh {
                action: MeshAction::Blockers { paths },
            } => {
                assert_eq!(paths, vec!["src/store/mesh.rs", "README.md"]);
            }
            _ => panic!("expected mesh blockers command"),
        }
    }

    #[test]
    fn parse_wait_path_flag() {
        let cli = Cli::parse_from([
            "tak",
            "wait",
            "--path",
            "src/store/mesh.rs",
            "--timeout",
            "5",
        ]);
        match cli.command {
            Commands::Wait {
                path,
                on_task,
                timeout,
            } => {
                assert_eq!(path.as_deref(), Some("src/store/mesh.rs"));
                assert!(on_task.is_none());
                assert_eq!(timeout, Some(5));
            }
            _ => panic!("expected wait command"),
        }
    }

    #[test]
    fn parse_wait_on_task_flag() {
        let cli = Cli::parse_from(["tak", "wait", "--on-task", "42"]);
        match cli.command {
            Commands::Wait {
                path,
                on_task,
                timeout,
            } => {
                assert!(path.is_none());
                assert_eq!(on_task.as_deref(), Some("42"));
                assert!(timeout.is_none());
            }
            _ => panic!("expected wait command"),
        }
    }

    #[test]
    fn parse_work_defaults_to_start_resume_action() {
        let cli = Cli::parse_from(["tak", "work", "--tag", "cli", "--limit", "3"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert!(action.is_none());
                assert!(assignee.is_none());
                assert_eq!(tag.as_deref(), Some("cli"));
                assert_eq!(limit, Some(3));
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_start_action_variant() {
        let cli = Cli::parse_from(["tak", "work", "start", "--verify", "local"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Start));
                assert!(assignee.is_none());
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert_eq!(verify, Some(WorkVerifyMode::Local));
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_start_with_strategy() {
        let cli = Cli::parse_from(["tak", "work", "start", "--strategy", "epic_closeout"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Start));
                assert!(assignee.is_none());
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert_eq!(strategy, Some(WorkClaimStrategy::EpicCloseout));
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_start_with_verbosity() {
        let cli = Cli::parse_from(["tak", "work", "start", "--verbosity", "high"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Start));
                assert!(assignee.is_none());
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert_eq!(verbosity, Some(WorkCoordinationVerbosity::High));
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_start_with_force_reclaim() {
        let cli = Cli::parse_from(["tak", "work", "start", "--force-reclaim"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Start));
                assert!(assignee.is_none());
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_status_action() {
        let cli = Cli::parse_from(["tak", "work", "status", "--assignee", "agent-1"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Status));
                assert_eq!(assignee.as_deref(), Some("agent-1"));
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_stop_action() {
        let cli = Cli::parse_from(["tak", "work", "stop"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Stop));
                assert!(assignee.is_none());
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(!pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_work_done_action_with_pause() {
        let cli = Cli::parse_from(["tak", "work", "done", "--pause", "--assignee", "agent-1"]);
        match cli.command {
            Commands::Work {
                action,
                assignee,
                tag,
                limit,
                verify,
                strategy,
                verbosity,
                force_reclaim,
                pause,
            } => {
                assert_eq!(action, Some(WorkAction::Done));
                assert_eq!(assignee.as_deref(), Some("agent-1"));
                assert!(tag.is_none());
                assert!(limit.is_none());
                assert!(verify.is_none());
                assert!(strategy.is_none());
                assert!(verbosity.is_none());
                assert!(!force_reclaim);
                assert!(pause);
            }
            _ => panic!("expected work command"),
        }
    }

    #[test]
    fn parse_metrics_burndown_defaults() {
        let cli = Cli::parse_from(["tak", "metrics", "burndown"]);
        match cli.command {
            Commands::Metrics { action } => match action {
                MetricsAction::Burndown {
                    from,
                    to,
                    bucket,
                    kind,
                    tag,
                    assignee,
                    children_of,
                    include_cancelled,
                } => {
                    assert!(from.is_none());
                    assert!(to.is_none());
                    assert_eq!(bucket, MetricsBucket::Day);
                    assert!(kind.is_none());
                    assert!(tag.is_empty());
                    assert!(assignee.is_none());
                    assert!(children_of.is_none());
                    assert!(!include_cancelled);
                }
                _ => panic!("expected burndown metrics action"),
            },
            _ => panic!("expected metrics command"),
        }
    }

    #[test]
    fn parse_metrics_burndown_with_filters() {
        let cli = Cli::parse_from([
            "tak",
            "metrics",
            "burndown",
            "--from",
            "2026-01-01",
            "--to",
            "2026-01-31",
            "--bucket",
            "week",
            "--kind",
            "task",
            "--tag",
            "metrics,cli",
            "--assignee",
            "agent-1",
            "--children-of",
            "123",
            "--include-cancelled",
        ]);

        match cli.command {
            Commands::Metrics { action } => match action {
                MetricsAction::Burndown {
                    from,
                    to,
                    bucket,
                    kind,
                    tag,
                    assignee,
                    children_of,
                    include_cancelled,
                } => {
                    assert_eq!(from, NaiveDate::from_ymd_opt(2026, 1, 1));
                    assert_eq!(to, NaiveDate::from_ymd_opt(2026, 1, 31));
                    assert_eq!(bucket, MetricsBucket::Week);
                    assert_eq!(kind, Some(Kind::Task));
                    assert_eq!(tag, vec!["metrics", "cli"]);
                    assert_eq!(assignee.as_deref(), Some("agent-1"));
                    assert_eq!(children_of.as_deref(), Some("123"));
                    assert!(include_cancelled);
                }
                _ => panic!("expected burndown metrics action"),
            },
            _ => panic!("expected metrics command"),
        }
    }

    #[test]
    fn parse_metrics_completion_time_defaults() {
        let cli = Cli::parse_from(["tak", "metrics", "completion-time"]);
        match cli.command {
            Commands::Metrics { action } => match action {
                MetricsAction::CompletionTime {
                    from,
                    to,
                    bucket,
                    metric,
                    kind,
                    tag,
                    assignee,
                    children_of,
                    include_cancelled,
                } => {
                    assert!(from.is_none());
                    assert!(to.is_none());
                    assert_eq!(bucket, MetricsBucket::Day);
                    assert_eq!(metric, CompletionMetric::Cycle);
                    assert!(kind.is_none());
                    assert!(tag.is_empty());
                    assert!(assignee.is_none());
                    assert!(children_of.is_none());
                    assert!(!include_cancelled);
                }
                _ => panic!("expected completion-time metrics action"),
            },
            _ => panic!("expected metrics command"),
        }
    }

    #[test]
    fn parse_metrics_completion_time_with_filters() {
        let cli = Cli::parse_from([
            "tak",
            "metrics",
            "completion-time",
            "--from",
            "2026-02-01",
            "--to",
            "2026-02-28",
            "--bucket",
            "week",
            "--metric",
            "lead",
            "--kind",
            "task",
            "--tag",
            "metrics,completion",
            "--assignee",
            "agent-2",
            "--children-of",
            "42",
            "--include-cancelled",
        ]);

        match cli.command {
            Commands::Metrics { action } => match action {
                MetricsAction::CompletionTime {
                    from,
                    to,
                    bucket,
                    metric,
                    kind,
                    tag,
                    assignee,
                    children_of,
                    include_cancelled,
                } => {
                    assert_eq!(from, NaiveDate::from_ymd_opt(2026, 2, 1));
                    assert_eq!(to, NaiveDate::from_ymd_opt(2026, 2, 28));
                    assert_eq!(bucket, MetricsBucket::Week);
                    assert_eq!(metric, CompletionMetric::Lead);
                    assert_eq!(kind, Some(Kind::Task));
                    assert_eq!(tag, vec!["metrics", "completion"]);
                    assert_eq!(assignee.as_deref(), Some("agent-2"));
                    assert_eq!(children_of.as_deref(), Some("42"));
                    assert!(include_cancelled);
                }
                _ => panic!("expected completion-time metrics action"),
            },
            _ => panic!("expected metrics command"),
        }
    }

    #[test]
    fn parse_wait_rejects_missing_target() {
        let err = match Cli::try_parse_from(["tak", "wait", "--timeout", "3"]) {
            Ok(_) => panic!("expected clap parse error"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("required"));
    }

    #[test]
    fn parse_tree_rejects_both_positional_and_named_id() {
        let err = match Cli::try_parse_from(["tak", "tree", "123", "--id", "456"]) {
            Ok(_) => panic!("expected clap parse error"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("cannot be used with"));
    }
}

fn main() {
    let cli = Cli::parse();
    let format = if cli.pretty {
        Format::Pretty
    } else {
        cli.format
    };
    if let Err(e) = run(cli, format) {
        match format {
            Format::Json => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "error": e.code(),
                        "message": e.to_string()
                    })
                );
            }
            _ => eprintln!("error: {e}"),
        }
        std::process::exit(1);
    }
}
