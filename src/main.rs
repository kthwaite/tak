use clap::{Parser, Subcommand};
use tak::model::{Kind, Status};
use tak::output::Format;

#[derive(Parser)]
#[command(name = "tak", version, about = "Git-native task manager for agentic workflows")]
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
    Init,
    Create {
        title: String,
        #[arg(long, value_enum, default_value = "task")]
        kind: Kind,
        #[arg(long)]
        parent: Option<u64>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<u64>,
        #[arg(long, short)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tag: Vec<String>,
    },
    Delete {
        id: u64,
    },
    Show {
        id: u64,
    },
    List {
        #[arg(long, value_enum)]
        status: Option<Status>,
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long, conflicts_with = "blocked")]
        available: bool,
        #[arg(long, conflicts_with = "available")]
        blocked: bool,
        #[arg(long)]
        children_of: Option<u64>,
    },
    Edit {
        id: u64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, short)]
        description: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        #[arg(long, value_delimiter = ',')]
        tag: Option<Vec<String>>,
    },
    Start {
        id: u64,
        #[arg(long)]
        assignee: Option<String>,
    },
    Finish {
        id: u64,
    },
    Cancel {
        id: u64,
    },
    Claim {
        #[arg(long, required = true)]
        assignee: String,
        /// Only claim tasks with this tag
        #[arg(long)]
        tag: Option<String>,
    },
    Reopen {
        id: u64,
    },
    Unassign {
        id: u64,
    },
    Depend {
        id: u64,
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<u64>,
    },
    Undepend {
        id: u64,
        #[arg(long, required = true, value_delimiter = ',')]
        on: Vec<u64>,
    },
    Reparent {
        id: u64,
        #[arg(long, required = true)]
        to: u64,
    },
    Orphan {
        id: u64,
    },
    Tree {
        id: Option<u64>,
    },
    Next {
        #[arg(long)]
        assignee: Option<String>,
    },
    Reindex,
}

fn run(cli: Cli, format: Format) -> tak::error::Result<()> {
    if matches!(cli.command, Commands::Init) {
        let cwd = std::env::current_dir()?;
        return tak::commands::init::run(&cwd);
    }

    let root = tak::store::repo::find_repo_root()?;

    match cli.command {
        Commands::Init => unreachable!(),
        Commands::Create {
            title,
            kind,
            parent,
            depends_on,
            description,
            tag,
        } => {
            tak::commands::create::run(
                &root, title, kind, description, parent, depends_on, tag, format,
            )
        }
        Commands::Delete { id } => tak::commands::delete::run(&root, id, format),
        Commands::Show { id } => tak::commands::show::run(&root, id, format),
        Commands::List {
            status,
            kind,
            tag,
            assignee,
            available,
            blocked,
            children_of,
        } => tak::commands::list::run(
            &root, status, kind, tag, assignee, available, blocked, children_of, format,
        ),
        Commands::Edit {
            id,
            title,
            description,
            kind,
            tag,
        } => tak::commands::edit::run(&root, id, title, description, kind, tag, format),
        Commands::Start { id, assignee } => {
            tak::commands::lifecycle::start(&root, id, assignee, format)
        }
        Commands::Finish { id } => tak::commands::lifecycle::finish(&root, id, format),
        Commands::Cancel { id } => tak::commands::lifecycle::cancel(&root, id, format),
        Commands::Claim { assignee, tag } => {
            tak::commands::claim::run(&root, assignee, tag, format)
        }
        Commands::Reopen { id } => tak::commands::lifecycle::reopen(&root, id, format),
        Commands::Unassign { id } => tak::commands::lifecycle::unassign(&root, id, format),
        Commands::Depend { id, on } => tak::commands::deps::depend(&root, id, on, format),
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
    let format = if cli.pretty { Format::Pretty } else { cli.format };
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
