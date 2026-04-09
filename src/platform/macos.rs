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

    // Attempt to open the relevant preference pane
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();

    Ok(())
}
