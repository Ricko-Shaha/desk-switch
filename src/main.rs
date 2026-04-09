mod capture;
mod config;
mod discovery;
mod gui;
mod input;
mod platform;
mod protocol;
mod service;
mod viewer;
mod virtual_display;

use anyhow::Result;
use clap::{Parser, Subcommand};
use log::info;

use crate::config::{load_config, save_config, Config};
use crate::service::Service;

#[derive(Parser)]
#[command(name = "desk-switch")]
#[command(about = "Cross-platform KVM switch for multi-laptop setups")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// First-time setup: generate auth key, configure hostname
    Setup,

    /// Start the desk-switch daemon
    Start {
        /// Run as primary immediately
        #[arg(long)]
        primary: bool,

        /// Run as display immediately
        #[arg(long)]
        display: bool,
    },

    /// Show current status
    Status,

    /// Start as primary (capture screen, accept display connections)
    Primary,

    /// Start as display (connect to primary, show remote screen)
    Display,

    /// Switch roles (toggle between primary and display)
    Switch,

    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },

    /// List available displays/monitors
    Monitors,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set a config value
    Set {
        /// Config key (e.g., quality, monitor, max_fps)
        key: String,
        /// New value
        value: String,
    },
    /// Get a config value
    Get {
        /// Config key
        key: String,
    },
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let cli = Cli::parse();

    match cli.command {
        None => gui::launch_gui(),
        Some(Commands::Setup) => cmd_setup(),
        Some(Commands::Start { primary, display }) => {
            let role = if primary {
                Some("primary")
            } else if display {
                Some("display")
            } else {
                None
            };
            cmd_start(role)
        }
        Some(Commands::Status) => cmd_status(),
        Some(Commands::Primary) => cmd_start(Some("primary")),
        Some(Commands::Display) => cmd_start(Some("display")),
        Some(Commands::Switch) => cmd_switch(),
        Some(Commands::Config { action }) => cmd_config(action),
        Some(Commands::Monitors) => cmd_monitors(),
    }
}

fn cmd_setup() -> Result<()> {
    println!("=== Desk Switch Setup ===\n");

    let config = Config::default();
    save_config(&config)?;

    println!("Configuration created at: {}", config::config_path().display());
    println!();
    println!("  Hostname:       {}", config.hostname);
    println!("  Auth key:       {}", config.auth_key);
    println!("  Stream port:    {}", config.stream_port);
    println!("  Discovery port: {}", config.discovery_port);
    println!("  Capture quality: {}", config.capture_quality);
    println!("  Max FPS:        {}", config.max_fps);
    println!();
    println!("IMPORTANT: Copy the auth key to the other machine.");
    println!("On the other machine, run `desk-switch setup` then:");
    println!("  desk-switch config set auth_key {}", config.auth_key);
    println!();
    println!("To start:");
    println!("  desk-switch start --primary    # On the primary machine");
    println!("  desk-switch start --display    # On the display machine");

    // Platform-specific setup
    platform::setup_permissions()?;

    Ok(())
}

fn cmd_start(role_override: Option<&str>) -> Result<()> {
    let mut config = load_config()?;

    if let Some(role) = role_override {
        config.default_role = role.to_string();
    }

    info!("Starting desk-switch as {}", config.default_role);

    let svc = Service::new(config);
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(svc.run())?;

    Ok(())
}

fn cmd_status() -> Result<()> {
    let config = load_config()?;

    println!("=== Desk Switch Status ===\n");
    println!("  Hostname:        {}", config.hostname);
    println!("  Default role:    {}", config.default_role);
    println!("  Stream port:     {}", config.stream_port);
    println!("  Discovery port:  {}", config.discovery_port);
    println!("  Capture quality: {}", config.capture_quality);
    println!("  Capture monitor: {}", config.capture_monitor);
    println!("  Viewer monitor:  {}", config.viewer_monitor);
    println!("  Max FPS:         {}", config.max_fps);

    println!("\n  Displays:");
    for d in capture::list_displays() {
        println!("    {}", d);
    }

    Ok(())
}

fn cmd_switch() -> Result<()> {
    let mut config = load_config()?;
    let new_role = match config.default_role.as_str() {
        "primary" => "display",
        "display" => "primary",
        _ => "primary",
    };
    config.default_role = new_role.to_string();
    save_config(&config)?;

    println!("Role switched to: {}", new_role);
    println!("Restart desk-switch for the change to take effect:");
    println!("  desk-switch start");

    Ok(())
}

fn cmd_config(action: Option<ConfigAction>) -> Result<()> {
    let mut config = load_config()?;

    match action {
        None => {
            let json = serde_json::to_string_pretty(&config)?;
            println!("{}", json);
        }
        Some(ConfigAction::Set { key, value }) => {
            config.set_field(&key, &value)?;
            save_config(&config)?;
            println!("Set {} = {}", key, value);
        }
        Some(ConfigAction::Get { key }) => {
            let json = serde_json::to_value(&config)?;
            if let Some(val) = json.get(&key) {
                println!("{}", val);
            } else {
                println!("Unknown key: {}", key);
            }
        }
    }

    Ok(())
}

fn cmd_monitors() -> Result<()> {
    println!("Available displays:\n");
    for d in capture::list_displays() {
        println!("  {}", d);
    }
    Ok(())
}
