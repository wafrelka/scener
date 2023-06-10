use anyhow::Result;
use clap::Parser;

use scener::*;

fn main() -> Result<()> {
    Cli::parse().run()
}
