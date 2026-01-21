use anyhow::Result;
use clap::Parser;

use pm_execpolicy::execpolicycheck::ExecPolicyCheckCommand;

#[derive(Parser)]
#[command(name = "pm-execpolicy")]
enum Cli {
    Check(ExecPolicyCheckCommand),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli {
        Cli::Check(cmd) => cmd.run(),
    }
}
