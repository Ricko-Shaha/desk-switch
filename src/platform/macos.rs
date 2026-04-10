use anyhow::Result;
use log::{info, warn};
use std::process::Command;

pub fn setup_permissions() -> Result<()> {
    info!("Running macOS permissions setup");

    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();

    add_firewall_exception();

    Ok(())
}

/// Add the current binary as a firewall exception using osascript
/// so the user gets a native password dialog (sudo fails silently from GUI).
pub fn add_firewall_exception() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let exe_str = exe.to_string_lossy().to_string();

    let script = format!(
        "do shell script \"/usr/libexec/ApplicationFirewall/socketfilterfw --remove '{}' 2>/dev/null; \
         /usr/libexec/ApplicationFirewall/socketfilterfw --add '{}'; \
         /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp '{}'\" \
         with administrator privileges",
        exe_str, exe_str, exe_str
    );

    info!("Requesting firewall exception for {}", exe_str);

    match Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                info!("Firewall exception added successfully");
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("User canceled") || stderr.contains("-128") {
                    warn!("User canceled firewall prompt — incoming connections may be blocked");
                } else {
                    warn!("Firewall exception failed: {}", stderr.trim());
                }
            }
        }
        Err(e) => {
            warn!("Could not run osascript for firewall: {}", e);
        }
    }
}
