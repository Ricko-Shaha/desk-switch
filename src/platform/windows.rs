use anyhow::Result;
use std::process::Command;

pub fn setup_permissions() -> Result<()> {
    println!("\n=== Windows Firewall Setup ===\n");

    // Add firewall rules for desk-switch ports
    let ports = [9876u16, 9877];
    for port in &ports {
        let rule_name = format!("DeskSwitch-TCP-{}", port);
        let result = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={}", rule_name),
                "dir=in",
                "action=allow",
                "protocol=TCP",
                &format!("localport={}", port),
            ])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                println!("  Added firewall rule: {} (TCP port {})", rule_name, port);
            }
            _ => {
                println!(
                    "  Could not add firewall rule for port {}. You may need to run as Administrator.",
                    port
                );
            }
        }

        let rule_name_udp = format!("DeskSwitch-UDP-{}", port);
        let result = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={}", rule_name_udp),
                "dir=in",
                "action=allow",
                "protocol=UDP",
                &format!("localport={}", port),
            ])
            .output();

        match result {
            Ok(output) if output.status.success() => {
                println!(
                    "  Added firewall rule: {} (UDP port {})",
                    rule_name_udp, port
                );
            }
            _ => {
                println!(
                    "  Could not add UDP firewall rule for port {}. You may need to run as Administrator.",
                    port
                );
            }
        }
    }

    println!();
    println!("If rules failed, open an Administrator terminal and re-run `desk-switch setup`.");

    Ok(())
}
