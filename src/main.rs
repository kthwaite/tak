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

fn run() -> tak::error::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    match cli.command {
        Commands::Init => {
            tak::commands::init::run(&cwd)?;
        }
        Commands::Create {
            title,
            kind,
            parent,
            depends_on,
            description,
            tag,
        } => {
            tak::commands::create::run(
                &cwd,
                title,
                &kind,
                description,
                parent,
                depends_on,
                tag,
                cli.pretty,
            )?;
        }
        Commands::Show { id } => {
            tak::commands::show::run(&cwd, id, cli.pretty)?;
        }
        Commands::List { .. } => {
            eprintln!("list command not yet implemented");
        }
        Commands::Edit { .. } => {
            eprintln!("edit command not yet implemented");
        }
        Commands::Start { .. } => {
            eprintln!("start command not yet implemented");
        }
        Commands::Finish { .. } => {
            eprintln!("finish command not yet implemented");
        }
        Commands::Cancel { .. } => {
            eprintln!("cancel command not yet implemented");
        }
        Commands::Depend { .. } => {
            eprintln!("depend command not yet implemented");
        }
        Commands::Undepend { .. } => {
            eprintln!("undepend command not yet implemented");
        }
        Commands::Reparent { .. } => {
            eprintln!("reparent command not yet implemented");
        }
        Commands::Orphan { .. } => {
            eprintln!("orphan command not yet implemented");
        }
        Commands::Tree { .. } => {
            eprintln!("tree command not yet implemented");
        }
        Commands::Next { .. } => {
            eprintln!("next command not yet implemented");
        }
        Commands::Reindex => {
            eprintln!("reindex command not yet implemented");
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
