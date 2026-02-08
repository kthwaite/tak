use clap::{Parser, Subcommand};
use tak::model::{DepType, Estimate, Kind, Priority, Risk, Status};
use tak::output::Format;

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
        parent: Option<u64>,
        /// Task IDs this task depends on (comma-separated)
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<u64>,
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
        id: u64,
        /// Cascade: orphan children and remove incoming dependencies
        #[arg(long)]
        force: bool,
    },
    /// Display a single task
    Show {
        /// Task ID to show
        id: u64,
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
        children_of: Option<u64>,
        /// Filter by priority
        #[arg(long, value_enum)]
        priority: Option<Priority>,
    },
    /// Edit task fields
    Edit {
        /// Task ID to edit
        id: u64,
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
        id: u64,
        /// Who is working on it
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Set a task to done
    Finish {
        /// Task ID to finish
        id: u64,
    },
    /// Set a task to cancelled
    Cancel {
        /// Task ID to cancel
        id: u64,
        /// Reason for cancellation (recorded as last_error)
        #[arg(long)]
        reason: Option<String>,
    },
    /// Hand off an in-progress task back to pending for another agent
    Handoff {
        /// Task ID to hand off
        id: u64,
        /// Summary of progress so far (required)
        #[arg(long, required = true)]
        summary: String,
    },
    /// Atomically find and start the next available task
    Claim {
        /// Who is claiming the task
        #[arg(long, required = true)]
        assignee: String,
        /// Only claim tasks with this tag
        #[arg(long)]
        tag: Option<String>,
    },
    /// Reopen a done or cancelled task back to pending
    Reopen {
        /// Task ID to reopen
        id: u64,
    },
    /// Clear a task's assignee without changing status
    Unassign {
        /// Task ID to unassign
        id: u64,
    },
    /// Add dependency edges (task cannot start until deps are done)
    Depend {
        /// Task ID that will gain dependencies
        id: u64,
        /// IDs of tasks it depends on (comma-separated)
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<u64>,
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
        id: u64,
        /// IDs of dependencies to remove (comma-separated)
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<u64>,
    },
    /// Change a task's parent
    Reparent {
        /// Task ID to reparent
        id: u64,
        /// New parent task ID
        #[arg(long, required = true)]
        to: u64,
    },
    /// Remove a task's parent (make it a root task)
    Orphan {
        /// Task ID to orphan
        id: u64,
    },
    /// Display the parent-child task hierarchy
    Tree {
        /// Root task ID (omit for full tree)
        id: Option<u64>,
    },
    /// Show the next available task without claiming it
    Next {
        /// Include tasks assigned to this person
        #[arg(long)]
        assignee: Option<String>,
    },
    /// Rebuild the SQLite index from task files
    Reindex,
    /// Install Claude Code integration (hooks + optional plugin)
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
        /// Also write the full plugin directory to CWD
        #[arg(long)]
        plugin: bool,
    },
    /// Validate tak installation and report issues
    Doctor {
        /// Auto-fix what can be fixed (reindex if stale, etc.)
        #[arg(long)]
        fix: bool,
    },
}

fn run(cli: Cli, format: Format) -> tak::error::Result<()> {
    // Commands that don't require a repo
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
        } => {
            return tak::commands::setup::run(*global, *check, *remove, *plugin, format);
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
        Commands::Delete { id, force } => tak::commands::delete::run(&root, id, force, format),
        Commands::Show { id } => tak::commands::show::run(&root, id, format),
        Commands::List {
            status,
            kind,
            tag,
            assignee,
            available,
            blocked,
            children_of,
            priority,
        } => tak::commands::list::run(
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
        ),
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
            format,
        ),
        Commands::Start { id, assignee } => {
            tak::commands::lifecycle::start(&root, id, assignee, format)
        }
        Commands::Finish { id } => tak::commands::lifecycle::finish(&root, id, format),
        Commands::Cancel { id, reason } => {
            tak::commands::lifecycle::cancel(&root, id, reason, format)
        }
        Commands::Handoff { id, summary } => {
            tak::commands::lifecycle::handoff(&root, id, summary, format)
        }
        Commands::Claim { assignee, tag } => {
            tak::commands::claim::run(&root, assignee, tag, format)
        }
        Commands::Reopen { id } => tak::commands::lifecycle::reopen(&root, id, format),
        Commands::Unassign { id } => tak::commands::lifecycle::unassign(&root, id, format),
        Commands::Depend {
            id,
            on,
            dep_type,
            reason,
        } => tak::commands::deps::depend(&root, id, on, dep_type, reason, format),
        Commands::Undepend { id, on } => tak::commands::deps::undepend(&root, id, on, format),
        Commands::Reparent { id, to } => tak::commands::deps::reparent(&root, id, to, format),
        Commands::Orphan { id } => tak::commands::deps::orphan(&root, id, format),
        Commands::Tree { id } => tak::commands::tree::run(&root, id, format),
        Commands::Next { assignee } => tak::commands::next::run(&root, assignee, format),
        Commands::Reindex => tak::commands::reindex::run(&root),
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
