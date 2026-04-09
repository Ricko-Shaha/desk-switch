use anyhow::{anyhow, Result};
use log::{info, warn};
use std::io::Write;
use std::path::PathBuf;

const VDD_PIPE_NAME: &str = r"\\.\pipe\DeskSwitchVDD";
const VDD_DRIVER_DIR: &str = r"C:\ProgramData\DeskSwitch\Driver";

pub struct WinVirtualMonitor {
    _width: u32,
    _height: u32,
}

impl WinVirtualMonitor {
    pub fn create(width: u32, height: u32, refresh_rate: u32) -> Result<Self> {
        if !Self::is_driver_installed() {
            return Err(anyhow!(
                "Virtual Display Driver is not installed.\n\
                 Install manually:\n\
                 1. Download from: https://github.com/VirtualDrivers/Virtual-Display-Driver/releases\n\
                 2. Extract and run as Admin: pnputil /add-driver VirtualDisplayDriver.inf /install\n\
                 3. You may need: bcdedit /set testsigning on (then reboot)"
            ));
        }

        info!(
            "Creating virtual display {}x{}@{}Hz via VDD",
            width, height, refresh_rate
        );

        let config = serde_json::json!({
            "monitors": [{
                "width": width,
                "height": height,
                "refresh_rate": refresh_rate,
                "name": "Desk Switch Virtual Display"
            }]
        });

        let config_dir = PathBuf::from(VDD_DRIVER_DIR);
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("monitors.json");
        std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;

        Self::signal_driver_reload()?;

        info!("Virtual display configuration written to {:?}", config_path);

        Ok(Self {
            _width: width,
            _height: height,
        })
    }

    pub fn destroy(&mut self) -> Result<()> {
        let config_dir = PathBuf::from(VDD_DRIVER_DIR);
        let config_path = config_dir.join("monitors.json");
        let empty = serde_json::json!({ "monitors": [] });
        let _ = std::fs::write(&config_path, serde_json::to_string_pretty(&empty)?);
        let _ = Self::signal_driver_reload();
        info!("Virtual display removed");
        Ok(())
    }

    fn is_driver_installed() -> bool {
        let driver_dir = PathBuf::from(VDD_DRIVER_DIR);
        if driver_dir.exists() {
            return true;
        }

        match std::process::Command::new("pnputil")
            .args(["/enum-devices", "/class", "Display"])
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains("Virtual Display") || stdout.contains("IddSample")
            }
            Err(_) => false,
        }
    }

    fn signal_driver_reload() -> Result<()> {
        match std::fs::OpenOptions::new()
            .write(true)
            .open(VDD_PIPE_NAME)
        {
            Ok(mut pipe) => {
                let _ = pipe.write_all(b"RELOAD\n");
                let _ = pipe.flush();
                info!("Signaled VDD driver to reload config");
                Ok(())
            }
            Err(_) => {
                warn!("Could not signal VDD driver via pipe. Config will apply on next driver restart.");
                Self::restart_device()?;
                Ok(())
            }
        }
    }

    fn restart_device() -> Result<()> {
        let result = std::process::Command::new("powershell")
            .args([
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Get-PnpDevice | Where-Object { $_.FriendlyName -like '*Virtual Display*' } | \
                 Disable-PnpDevice -Confirm:$false; \
                 Start-Sleep -Seconds 1; \
                 Get-PnpDevice | Where-Object { $_.FriendlyName -like '*Virtual Display*' } | \
                 Enable-PnpDevice -Confirm:$false",
            ])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                info!("VDD device restarted successfully");
                Ok(())
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("Device restart warning: {}", stderr);
                Ok(())
            }
            Err(e) => {
                warn!("Could not restart VDD device: {}", e);
                Ok(())
            }
        }
    }
}

impl Drop for WinVirtualMonitor {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}
