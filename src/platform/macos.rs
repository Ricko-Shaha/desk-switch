use anyhow::Result;
use std::process::Command;

pub fn setup_permissions() -> Result<()> {
    println!("\n=== macOS Permissions ===\n");
    println!("desk-switch needs Accessibility permissions to capture and simulate input.");
    println!("If prompted, grant access in:");
    println!("  System Settings > Privacy & Security > Accessibility");
    println!();
    println!("For screen capture, you may also need to grant access in:");
    println!("  System Settings > Privacy & Security > Screen Recording");
    println!();

    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();

    add_firewall_exception();

    Ok(())
}

fn add_firewall_exception() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let exe_str = exe.to_string_lossy().to_string();

    // Remove existing rule, then add as allowed
    let _ = Command::new("sudo")
        .args([
            "/usr/libexec/ApplicationFirewall/socketfilterfw",
            "--remove",
            &exe_str,
        ])
        .output();

    let _ = Command::new("sudo")
        .args([
            "/usr/libexec/ApplicationFirewall/socketfilterfw",
            "--add",
            &exe_str,
        ])
        .output();

    let _ = Command::new("sudo")
        .args([
            "/usr/libexec/ApplicationFirewall/socketfilterfw",
            "--unblockapp",
            &exe_str,
        ])
        .output();
}
