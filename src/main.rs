use clap::{Parser, Subcommand};
use tak::model::{DepType, Estimate, Kind, LearningCategory, Priority, Risk, Status};
use tak::output::Format;
use tak::store::blackboard::BlackboardStatus;

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
    /// Initialize a new .tak/ directory in the current repository
    Init,
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
    /// Display a single task
    Show {
        /// Task ID to show
        id: String,
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
    /// Set a task to in_progress
    Start {
        /// Task ID to start
        id: String,
        /// Who is working on it
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Set a task to done
    Finish {
        /// Task ID to finish
        id: String,
    },
    /// Set a task to cancelled
    Cancel {
        /// Task ID to cancel
        id: String,
        /// Reason for cancellation (recorded as last_error)
        #[arg(long)]
        reason: Option<String>,
    },
    /// Hand off an in-progress task back to pending for another agent
    Handoff {
        /// Task ID to hand off
        id: String,
        /// Summary of progress so far (required)
        #[arg(long, required = true)]
        summary: String,
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
    /// Reopen a done or cancelled task back to pending
    Reopen {
        /// Task ID to reopen
        id: String,
    },
    /// Clear a task's assignee without changing status
    Unassign {
        /// Task ID to unassign
        id: String,
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
    /// Remove dependency edges
    Undepend {
        /// Task ID to remove dependencies from
        id: String,
        /// IDs of dependencies to remove (comma-separated)
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<String>,
    },
    /// Change a task's parent
    Reparent {
        /// Task ID to reparent
        id: String,
        /// New parent task ID
        #[arg(long, required = true)]
        to: String,
    },
    /// Remove a task's parent (make it a root task)
    Orphan {
        /// Task ID to orphan
        id: String,
    },
    /// Display the parent-child task hierarchy
    Tree {
        /// Root task ID (omit for full tree)
        id: Option<String>,
    },
    /// Show the next available task without claiming it
    Next {
        /// Include tasks assigned to this person
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Run verification commands from task contract
    Verify {
        /// Task ID
        id: String,
    },
    /// Display history log for a task
    Log {
        /// Task ID
        id: String,
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
    /// Manage learnings (add, list, show, edit, remove, suggest)
    Learn {
        #[command(subcommand)]
        action: LearnAction,
    },
    /// Multi-agent coordination mesh
    Mesh {
        #[command(subcommand)]
        action: MeshAction,
    },
    /// Shared coordination blackboard
    Blackboard {
        #[command(subcommand)]
        action: BlackboardAction,
    },
    /// Diagnose /tak workflow friction and record therapist observations
    Therapist {
        #[command(subcommand)]
        action: TherapistAction,
    },
    /// Migrate legacy numeric task IDs (scaffold; dry-run by default)
    MigrateIds {
        /// Preview migration preflight (default when --apply is not provided)
        #[arg(long, conflicts_with = "apply")]
        dry_run: bool,
        /// Apply migration changes (preflight-only scaffold for now)
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,
        /// Skip in-progress task safety gate
        #[arg(long)]
        force: bool,
    },
    /// Rebuild the SQLite index from task files
    Reindex,
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
    /// Validate tak installation and report issues
    Doctor {
        /// Auto-fix what can be fixed (reindex if stale, etc.)
        #[arg(long)]
        fix: bool,
    },
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
    },
    /// Broadcast a message to all agents
    Broadcast {
        /// Sender name
        #[arg(long)]
        from: String,
        /// Message text
        #[arg(long)]
        message: String,
    },
    /// Read inbox messages
    Inbox {
        /// Agent name
        #[arg(long)]
        name: String,
        /// Acknowledge (delete) messages after reading
        #[arg(long)]
        ack: bool,
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
        /// Message text
        #[arg(long)]
        message: String,
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
        Commands::Handoff { id, summary } => tak::commands::lifecycle::handoff(
            &root,
            resolve_task_id_arg(&root, id)?,
            summary,
            format,
        ),
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
        Commands::Tree { id } => {
            tak::commands::tree::run(&root, resolve_optional_task_id_arg(&root, id)?, format)
        }
        Commands::Next { assignee } => tak::commands::next::run(&root, assignee, format),
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
        Commands::Mesh { action } => match action {
            MeshAction::Join { name, session_id } => {
                tak::commands::mesh::join(&root, name.as_deref(), session_id.as_deref(), format)
            }
            MeshAction::Leave { name } => {
                tak::commands::mesh::leave(&root, name.as_deref(), format)
            }
            MeshAction::List => tak::commands::mesh::list(&root, format),
            MeshAction::Send { from, to, message } => {
                tak::commands::mesh::send(&root, &from, &to, &message, format)
            }
            MeshAction::Broadcast { from, message } => {
                tak::commands::mesh::broadcast(&root, &from, &message, format)
            }
            MeshAction::Inbox { name, ack } => {
                tak::commands::mesh::inbox(&root, &name, ack, format)
            }
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
                tag,
                task_ids,
            } => tak::commands::blackboard::post(
                &root,
                &from,
                &message,
                tag,
                resolve_task_id_args(&root, task_ids)?,
                format,
            ),
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
            force,
        } => {
            let dry_run = dry_run || !apply;
            tak::commands::migrate_ids::run(&root, dry_run, force, format)
        }
        Commands::Reindex => tak::commands::reindex::run(&root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tak::error::TakError;
    use tak::model::{Contract, Kind, Planning};
    use tak::store::files::FileStore;
    use tempfile::tempdir;

    fn init_repo_with_tasks(count: usize) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let store = FileStore::init(dir.path()).unwrap();
        for i in 0..count {
            store
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
        }
        dir
    }

    #[test]
    fn resolve_task_id_arg_accepts_canonical_hex_input() {
        let dir = init_repo_with_tasks(1);

        let resolved = resolve_task_id_arg(dir.path(), "0000000000000001".into()).unwrap();
        assert_eq!(resolved, 1);
    }

    #[test]
    fn resolve_task_id_arg_surfaces_ambiguous_prefix() {
        let dir = init_repo_with_tasks(2);

        let err = resolve_task_id_arg(dir.path(), "00000000000000".into()).unwrap_err();
        assert!(matches!(err, TakError::TaskIdAmbiguous(_, _)));
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
