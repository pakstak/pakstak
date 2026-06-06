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
        /// User-provided app alias to install under ~/.var/pakstak/apps/<alias>.
        alias: String,
        /// Image reference to install, for example alpine:latest or ghcr.io/org/app:tag.
        image: String,
    },
    /// Run a command inside an installed image rootfs.
    Run {
        /// Installed app alias from ~/.var/pakstak/apps/<alias>.
        app_alias: String,
        /// Command and arguments to execute inside the container.
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let ctx = Context::new().context("failed to initialize application context")?;

    match cli.command {
        Command::Install { alias, image } => install::install(&ctx, &alias, &image),
        Command::Run { app_alias, command } => run::run(&ctx, &app_alias, command),
    }
}
