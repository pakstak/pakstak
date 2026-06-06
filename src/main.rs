mod auth;
mod digest;
mod fetch;
mod install;
mod manifest;
mod prune;
mod reference;
mod run;
mod storage;
mod uninstall;
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
        /// User-defined container name,
        /// must be unique and only contain ASCII letters, numbers, dots, underscores, and dashes.
        container: String,
        /// Image reference to install, for example ghcr.io/org/container:tag.
        image: String,
    },
    /// Remove an installed container without deleting cached layers or manifests.
    Uninstall {
        /// Installed container name.
        container: String,
    },
    /// Update installed containers to their latest manifest and layers.
    Update {
        /// Optional installed containers to update. If omitted, all containers are updated.
        containers: Vec<String>,
    },
    /// Remove cached manifests and layers that are not used by installed containers.
    Prune,
    /// Run a command inside an installed image rootfs.
    Run {
        /// Installed container name.
        container: String,
        /// Command and arguments that are passed to the Bubblewrap.
        #[arg(required = true, last = true)]
        command: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Install { container, image } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            install::install(&storage, &container, &image)
        }
        Command::Uninstall { container } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            uninstall::uninstall(&storage, &container)
        }
        Command::Update { containers } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            update::update(&storage, containers)
        }
        Command::Prune => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            prune::prune(&storage)
        }
        Command::Run { container, command } => {
            let storage = Storage::new().context("failed to initialize storage")?;
            run::run(&storage, &container, command)
        }
    }
}
