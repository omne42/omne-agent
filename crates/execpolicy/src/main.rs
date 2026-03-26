use anyhow::Result;
use clap::Parser;

use omne_execpolicy::ExecPolicyCheckCommand;

#[derive(Parser)]
#[command(name = "omne-execpolicy")]
enum Cli {
    Check(ExecPolicyCheckCommand),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli {
        Cli::Check(cmd) => cmd.run(),
    }
}
