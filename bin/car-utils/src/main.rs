mod cat;
mod error;
mod ls;
mod pack;
mod unpack;
use clap::{Parser, Subcommand};

/// The short version information for car-utils.
///
/// - The latest version from Cargo.toml
///
/// # Example
///
/// ```text
/// v0.1.5
/// ```
pub(crate) const SHORT_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Parser)]
#[command(author, version = SHORT_VERSION, long_version = SHORT_VERSION, about = "car-utils", long_about = None)]
struct Cli {
    /// The command to run
    #[clap(subcommand)]
    command: Commands,
}

/// Commands to be executed
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Pack files into a CAR.
    #[command(name = "pack")]
    Pack(pack::PackCommand),

    /// Unpack files and directories from a CAR.
    #[command(name = "unpack")]
    Unpack(unpack::UnpackCommand),

    /// List the car files.
    #[command(name = "ls")]
    Ls(ls::LsCommand),

    /// List root CIDs from a CAR.
    #[command(name = "roots")]
    Roots(ls::LsCommand),

    /// View cid content from a car file.
    #[command(name = "cat")]
    Cat(cat::CatCommand),
}

fn main() {
    let opt = Cli::parse();
    if let Err(err) = match opt.command {
        Commands::Pack(command) => command.execute(),
        Commands::Unpack(command) => command.execute(),
        Commands::Ls(command) => command.execute(false),
        Commands::Roots(command) => command.execute(true),
        Commands::Cat(command) => command.execute(),
    } {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
