use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn get_session_dir() -> Result<PathBuf> {
    let base_dirs = xdg::BaseDirectories::with_prefix("scener")
        .context("could not locate xdg app data directory")?;
    Ok(base_dirs.get_data_file("sessions"))
}
