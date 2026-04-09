use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const DEFAULT_STREAM_PORT: u16 = 9876;
pub const DEFAULT_DISCOVERY_PORT: u16 = 9877;
pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Role {
    Idle = 0,
    Primary = 1,
    Display = 2,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Idle => write!(f, "idle"),
            Role::Primary => write!(f, "primary"),
            Role::Display => write!(f, "display"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Handshake {
        version: u16,
        auth_hash: Vec<u8>,
        hostname: String,
    },
    HandshakeAck {
        accepted: bool,
    },
    Frame {
        width: u16,
        height: u16,
        jpeg_data: Vec<u8>,
    },
    MouseMove {
        x_ratio: f32,
        y_ratio: f32,
    },
    MouseClick {
        button: u8,
        pressed: bool,
        x_ratio: f32,
        y_ratio: f32,
    },
    MouseScroll {
        dx: i32,
        dy: i32,
    },
    KeyEvent {
        key_code: u32,
        pressed: bool,
    },
    RoleSwitch {
        new_role: u8,
    },
    Heartbeat {
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryPacket {
    pub hostname: String,
    pub ip: String,
    pub role: String,
    pub version: u16,
    pub stream_port: u16,
}

pub fn write_message_sync<W: Write>(writer: &mut W, msg: &Message) -> Result<()> {
    let payload = bincode::serialize(msg)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message_sync<R: Read>(reader: &mut R) -> Result<Message> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 10 * 1024 * 1024 {
        return Err(anyhow!("Message too large: {} bytes", len));
    }

    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    let msg: Message = bincode::deserialize(&payload)?;
    Ok(msg)
}
