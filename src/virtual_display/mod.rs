#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use anyhow::Result;

pub struct VirtualMonitor {
    #[cfg(target_os = "macos")]
    inner: macos::MacVirtualMonitor,
    #[cfg(target_os = "windows")]
    inner: windows::WinVirtualMonitor,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    inner: (),
    pub display_index: usize,
}

impl VirtualMonitor {
    pub fn create(width: u32, height: u32, refresh_rate: u32) -> Result<Self> {
        let displays_before = crate::capture::list_displays();

        #[cfg(target_os = "macos")]
        let inner = macos::MacVirtualMonitor::create(width, height, refresh_rate)?;

        #[cfg(target_os = "windows")]
        let inner = windows::WinVirtualMonitor::create(width, height, refresh_rate)?;

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (width, height, refresh_rate);
            anyhow::bail!("Virtual displays are not supported on this platform");
        }

        std::thread::sleep(std::time::Duration::from_secs(2));

        let displays_after = crate::capture::list_displays();
        let display_index = find_new_display(&displays_before, &displays_after)
            .unwrap_or(displays_after.len().saturating_sub(1));

        log::info!(
            "Virtual monitor created ({}x{}@{}Hz) → display index {}",
            width, height, refresh_rate, display_index
        );

        Ok(Self {
            inner,
            display_index,
        })
    }

    pub fn destroy(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        self.inner.destroy()?;
        #[cfg(target_os = "windows")]
        self.inner.destroy()?;
        log::info!("Virtual monitor destroyed");
        Ok(())
    }
}

impl Drop for VirtualMonitor {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}

fn find_new_display(before: &[String], after: &[String]) -> Option<usize> {
    for (i, d) in after.iter().enumerate() {
        if !before.contains(d) {
            return Some(i);
        }
    }
    if after.len() > before.len() {
        return Some(after.len() - 1);
    }
    None
}
