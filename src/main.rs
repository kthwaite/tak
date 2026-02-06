use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tak", version, about = "Git-native task manager for agentic workflows")]
struct Cli {
    #[arg(long, global = true)]
    pretty: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    Create {
        title: String,
        #[arg(long, default_value = "task")]
        kind: String,
        #[arg(long)]
        parent: Option<u64>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<u64>,
        #[arg(long, short)]
        description: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tag: Vec<String>,
    },
    Show {
        id: u64,
    },
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        available: bool,
        #[arg(long)]
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
        #[arg(long)]
        kind: Option<String>,
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

fn run(cli: Cli) -> tak::error::Result<()> {
    let pretty = cli.pretty;

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
                &root, title, &kind, description, parent, depends_on, tag, pretty,
            )
        }
        Commands::Show { id } => tak::commands::show::run(&root, id, pretty),
        Commands::List {
            status,
            kind,
            tag,
            assignee,
            available,
            blocked,
            children_of,
        } => tak::commands::list::run(
            &root, status, kind, tag, assignee, available, blocked, children_of, pretty,
        ),
        Commands::Edit {
            id,
            title,
            description,
            kind,
            tag,
        } => tak::commands::edit::run(&root, id, title, description, kind, tag, pretty),
        Commands::Start { id, assignee } => {
            tak::commands::lifecycle::start(&root, id, assignee, pretty)
        }
        Commands::Finish { id } => tak::commands::lifecycle::finish(&root, id, pretty),
        Commands::Cancel { id } => tak::commands::lifecycle::cancel(&root, id, pretty),
        Commands::Depend { id, on } => tak::commands::deps::depend(&root, id, on, pretty),
        Commands::Undepend { id, on } => tak::commands::deps::undepend(&root, id, on, pretty),
        Commands::Reparent { id, to } => tak::commands::deps::reparent(&root, id, to, pretty),
        Commands::Orphan { id } => tak::commands::deps::orphan(&root, id, pretty),
        Commands::Tree { id } => tak::commands::tree::run(&root, id, pretty),
        Commands::Next { assignee } => tak::commands::next::run(&root, assignee, pretty),
        Commands::Reindex => tak::commands::reindex::run(&root),
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
