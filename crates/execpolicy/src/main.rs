use anyhow::Result;
use clap::Parser;

use omne_agent_execpolicy::execpolicycheck::ExecPolicyCheckCommand;

#[derive(Parser)]
#[command(name = "omne-agent-execpolicy")]
enum Cli {
    Check(ExecPolicyCheckCommand),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli {
        Cli::Check(cmd) => cmd.run(),
    }
}
