use anyhow::{Context, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

use crate::protocol::{DEFAULT_DISCOVERY_PORT, DEFAULT_STREAM_PORT};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hostname: String,
    pub default_role: String,
    pub auth_key: String,
    pub stream_port: u16,
    pub discovery_port: u16,
    pub capture_quality: u8,
    pub capture_monitor: usize,
    pub viewer_monitor: usize,
    pub max_fps: u32,
}

impl Default for Config {
    fn default() -> Self {
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        Self {
            hostname,
            default_role: "idle".to_string(),
            auth_key: generate_auth_key(),
            stream_port: DEFAULT_STREAM_PORT,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            capture_quality: 60,
            capture_monitor: 0,
            viewer_monitor: 0,
            max_fps: 30,
        }
    }
}

impl Config {
    pub fn auth_hash(&self) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(self.auth_key.as_bytes());
        hasher.finalize().to_vec()
    }

    pub fn set_field(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "hostname" => self.hostname = value.to_string(),
            "default_role" => {
                match value {
                    "idle" | "primary" | "display" => self.default_role = value.to_string(),
                    _ => anyhow::bail!("Invalid role: {}. Must be idle, primary, or display", value),
                }
            }
            "auth_key" => self.auth_key = value.to_string(),
            "stream_port" => self.stream_port = value.parse().context("Invalid port number")?,
            "discovery_port" => self.discovery_port = value.parse().context("Invalid port number")?,
            "capture_quality" => {
                let q: u8 = value.parse().context("Invalid quality (1-100)")?;
                if !(1..=100).contains(&q) {
                    anyhow::bail!("Quality must be between 1 and 100");
                }
                self.capture_quality = q;
            }
            "capture_monitor" => self.capture_monitor = value.parse().context("Invalid monitor index")?,
            "viewer_monitor" => self.viewer_monitor = value.parse().context("Invalid monitor index")?,
            "max_fps" => {
                let fps: u32 = value.parse().context("Invalid FPS value")?;
                if !(1..=60).contains(&fps) {
                    anyhow::bail!("FPS must be between 1 and 60");
                }
                self.max_fps = fps;
            }
            _ => anyhow::bail!("Unknown config key: {}", key),
        }
        Ok(())
    }
}

pub fn config_dir() -> PathBuf {
    let home = dirs::home_dir().expect("Cannot determine home directory");
    home.join(".desk-switch")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load_config() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        anyhow::bail!(
            "Config not found. Run `desk-switch setup` first.\n  Expected at: {}",
            path.display()
        );
    }
    let data = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let config: Config =
        serde_json::from_str(&data).with_context(|| "Failed to parse config JSON")?;
    Ok(config)
}

pub fn save_config(config: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create config dir: {}", dir.display()))?;

    let path = config_path();
    let data = serde_json::to_string_pretty(config)?;
    fs::write(&path, data)
        .with_context(|| format!("Failed to write config at {}", path.display()))?;
    Ok(())
}

fn generate_auth_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    hex::encode(bytes)
}
