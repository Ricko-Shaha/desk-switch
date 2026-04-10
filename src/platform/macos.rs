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

/// Ad-hoc code sign the binary so macOS firewall will respect the exception.
/// Unsigned binaries are silently blocked regardless of socketfilterfw rules.
fn codesign_binary(exe_path: &str) {
    match Command::new("codesign")
        .args(["--force", "--sign", "-", exe_path])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                info!("Binary ad-hoc signed: {}", exe_path);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("is already signed") {
                    info!("Binary already signed");
                } else {
                    warn!("codesign failed: {}", stderr.trim());
                }
            }
        }
        Err(e) => warn!("Could not run codesign: {}", e),
    }
}

/// Add the current binary as a firewall exception.
/// Steps: ad-hoc sign → remove old rule → add + unblock via osascript admin prompt.
pub fn add_firewall_exception() {
    let exe = match std::env::current_exe() {
        Ok(p) => match p.canonicalize() {
            Ok(c) => c,
            Err(_) => p,
        },
        Err(_) => return,
    };
    let exe_str = exe.to_string_lossy().to_string();

    // Step 1: ad-hoc sign so the firewall recognizes the binary
    codesign_binary(&exe_str);

    // Step 2: add firewall exception via osascript (native admin prompt)
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

    // Step 3: verify the rule was applied
    match Command::new("/usr/libexec/ApplicationFirewall/socketfilterfw")
        .arg("--getappblocked")
        .arg(&exe_str)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            info!("Firewall status: {}", stdout.trim());
        }
        Err(_) => {}
    }
}
