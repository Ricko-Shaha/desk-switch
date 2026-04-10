#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
mod windows;

use anyhow::Result;

pub fn setup_permissions() -> Result<()> {
    #[cfg(target_os = "macos")]
    macos::setup_permissions()?;

    #[cfg(target_os = "windows")]
    windows::setup_permissions()?;

    Ok(())
}
