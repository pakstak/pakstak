mod auth;
mod digest;
mod fetch;
mod install;
mod manifest;
mod prune;
mod reference;
mod repair;
mod run;
mod storage;
mod switch;
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
        /// must be unique, start with an ASCII letter or number, and only contain ASCII letters,
        /// numbers, dots, underscores, and dashes.
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
    /// Switch an installed container to a specific manifest digest.
    Switch {
        /// Installed container name.
        container: String,
        /// Manifest digest to switch to, for example sha256:...
        digest: String,
    },
    /// Remove cached manifests and layers that are not used by installed containers.
    Prune,
    /// Repair installed containers and check cached manifests and layers.
    Repair,
    /// Run a command inside installed image rootfs layers.
    Run {
        /// Installed container names. Later containers are stacked above earlier containers.
        /// If a layer exists in both containers it is only binded once.
        #[arg(required = true)]
        containers: Vec<String>,
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
        Command::Switch { container, digest } => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            switch::switch(&storage, &container, &digest)
        }
        Command::Prune => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            prune::prune(&storage)
        }
        Command::Repair => {
            let storage = StorageMutable::new().context("failed to initialize mutable storage")?;
            repair::repair(&storage)
        }
        Command::Run {
            containers,
            command,
        } => {
            let storage = Storage::new().context("failed to initialize storage")?;
            run::run(&storage, containers, command)
        }
    }
}
