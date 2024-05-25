use anyhow::{Context, Result};

pub fn scan_line() -> Result<Option<String>> {
    eprint!("==> ");
    let line = match std::io::stdin().lines().next() {
        Some(c) => Some(c.context("could not read command from STDIN")?),
        None => None,
    };
    Ok(line)
}
