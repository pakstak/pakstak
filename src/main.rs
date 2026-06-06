mod context;
mod install;
mod manifest;
mod run;

use anyhow::Context as _;
use clap::{Parser, Subcommand};
use context::Context;

#[derive(Debug, Parser)]
#[command(version, about = "Fetch and unpack OCI image layers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Fetch an OCI image manifest and extract each layer.
    Install {
        /// Image reference to install, for example alpine:latest or ghcr.io/org/app:tag.
        image: String,
    },
    /// Run a command inside an installed image rootfs.
    Run {
        /// Installed manifest hash from ~/.var/pakstak/manifests/<hash>.json.
        manifest_hash: String,
        /// Command and arguments to execute inside the container.
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let ctx = Context::new().context("failed to initialize application context")?;

    match cli.command {
        Command::Install { image } => install::install(&ctx, &image),
        Command::Run {
            manifest_hash,
            command,
        } => run::run(&ctx, &manifest_hash, command),
    }
}
