mod digest;
mod fetch;
mod install;
mod manifest;
mod run;
mod storage;
mod update;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use storage::{Storage, StorageMutable};

#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Fetch and install an OCI image.
    Install {
        /// User-provided app alias,
        /// must be unique and only contain ASCII letters, numbers, dots, underscores, and dashes.
        alias: String,
        /// Image reference to install, for example alpine:latest or ghcr.io/org/app:tag.
        image: String,
    },
    /// Update installed apps to their latest manifest and layers.
    Update {
        /// Optional installed app aliases to update. If omitted, all apps are updated.
        aliases: Vec<String>,
    },
    /// Run a command inside an installed image rootfs.
    Run {
        /// Installed app alias.
        alias: String,
        /// Command and arguments that are passed to the Bubblewrap.
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Install { alias, image } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            install::install(&storage, &alias, &image)
        }
        Command::Update { aliases } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            update::update(&storage, aliases)
        }
        Command::Run { alias, command } => {
            let storage = Storage::new().context("failed to initialize storage")?;
            run::run(&storage, &alias, command)
        }
    }
}
