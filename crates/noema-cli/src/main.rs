use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "noema")]
#[command(about = "Noema local memory system")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
}

fn main() {
    let _cli = Cli::parse();
    println!("noema");
}
