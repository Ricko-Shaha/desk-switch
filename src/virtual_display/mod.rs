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
        #[cfg(target_os = "macos")]
        let inner = macos::MacVirtualMonitor::create(width, height, refresh_rate)?;

        #[cfg(target_os = "windows")]
        let inner = windows::WinVirtualMonitor::create(width, height, refresh_rate)?;

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (width, height, refresh_rate);
            anyhow::bail!("Virtual displays are not supported on this platform");
        }

        // Wait for OS to register the new display
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Find the display index using platform-specific ID matching
        #[cfg(target_os = "macos")]
        let display_index = {
            let target_id = inner.display_id;
            find_display_index_by_cg_id(target_id).unwrap_or_else(|| {
                log::warn!(
                    "Could not find display ID {} in active list, using last display",
                    target_id
                );
                let count = crate::capture::list_displays().len();
                count.saturating_sub(1)
            })
        };

        #[cfg(target_os = "windows")]
        let display_index = {
            let count = crate::capture::list_displays().len();
            count.saturating_sub(1)
        };

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let display_index = 0;

        log::info!(
            "Virtual monitor created ({}x{}@{}Hz) → display index {}",
            width,
            height,
            refresh_rate,
            display_index
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

/// Use CoreGraphics CGGetActiveDisplayList to find the scrap index for a given CGDirectDisplayID.
/// scrap uses the same ordering as CGGetActiveDisplayList internally.
#[cfg(target_os = "macos")]
fn find_display_index_by_cg_id(target_id: u32) -> Option<usize> {
    extern "C" {
        fn CGGetActiveDisplayList(
            max_displays: u32,
            active_displays: *mut u32,
            display_count: *mut u32,
        ) -> i32;
    }

    unsafe {
        let mut count: u32 = 0;
        let err = CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count);
        if err != 0 || count == 0 {
            return None;
        }

        let mut displays = vec![0u32; count as usize];
        let err = CGGetActiveDisplayList(count, displays.as_mut_ptr(), &mut count);
        if err != 0 {
            return None;
        }

        log::info!(
            "Active displays ({}): {:?}, looking for ID {}",
            count,
            &displays[..count as usize],
            target_id
        );

        displays[..count as usize]
            .iter()
            .position(|&id| id == target_id)
    }
}
