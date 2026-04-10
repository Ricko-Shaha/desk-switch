use anyhow::{anyhow, Result};
use log::{info, warn};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

pub struct MacVirtualMonitor {
    child: Option<Child>,
    pub display_id: u32,
}

impl MacVirtualMonitor {
    pub fn create(width: u32, height: u32, refresh_rate: u32) -> Result<Self> {
        let helper = Self::find_helper()?;

        info!(
            "Starting virtual display helper: {} {}x{}@{}",
            helper, width, height, refresh_rate
        );

        let mut child = Command::new(&helper)
            .arg(width.to_string())
            .arg(height.to_string())
            .arg(refresh_rate.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to start virtual display helper: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("No stdout from helper"))?;
        let reader = BufReader::new(stdout);

        let mut display_id = 0u32;
        for line in reader.lines() {
            let line = line?;
            if let Some(id_str) = line.strip_prefix("DISPLAY_ID=") {
                display_id = id_str
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("Invalid display ID: {}", id_str))?;
                break;
            }
        }

        if display_id == 0 {
            if let Some(ref mut stderr) = child.stderr {
                let mut err = String::new();
                let _ = std::io::Read::read_to_string(stderr, &mut err);
                if !err.is_empty() {
                    warn!("Helper stderr: {}", err.trim());
                }
            }
            let _ = child.kill();
            return Err(anyhow!(
                "Virtual display helper did not return a display ID. \
                 Make sure you're on macOS 14+ (Sonoma)."
            ));
        }

        info!("Virtual display created with ID {}", display_id);

        Ok(Self {
            child: Some(child),
            display_id,
        })
    }

    pub fn destroy(&mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(b"QUIT\n");
                let _ = stdin.flush();
            }
            match child.wait_timeout(std::time::Duration::from_secs(3)) {
                Ok(Some(_)) => {}
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
        self.child = None;
        Ok(())
    }

    fn find_helper() -> Result<String> {
        let exe_dir = std::env::current_exe()?
            .parent()
            .unwrap()
            .to_path_buf();

        let candidates = [
            exe_dir.join("virtual-display-helper"),
            exe_dir.join("../Resources/virtual-display-helper"),
            std::path::PathBuf::from("helpers/virtual-display-helper-bin"),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".desk-switch/virtual-display-helper"),
        ];

        for p in &candidates {
            if p.exists() {
                return Ok(p.to_string_lossy().to_string());
            }
        }

        Err(anyhow!(
            "virtual-display-helper not found.\n\
             Run Install.command to build it, or compile manually:\n\
             clang -framework Foundation -framework CoreGraphics \
             -o helpers/virtual-display-helper-bin helpers/virtual-display-helper.m"
        ))
    }
}

impl Drop for MacVirtualMonitor {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}

trait WaitTimeout {
    fn wait_timeout(
        &mut self,
        dur: std::time::Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl WaitTimeout for Child {
    fn wait_timeout(
        &mut self,
        dur: std::time::Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait()? {
                Some(status) => return Ok(Some(status)),
                None if start.elapsed() >= dur => return Ok(None),
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }
}
