use log::{debug, info, warn};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::{DiscoveryPacket, Role};

const BROADCAST_INTERVAL: Duration = Duration::from_secs(2);
const PEER_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub hostname: String,
    pub ip: String,
    pub role: String,
    pub stream_port: u16,
    pub last_seen: Instant,
}

pub type PeerMap = Arc<Mutex<HashMap<String, PeerInfo>>>;

pub fn new_peer_map() -> PeerMap {
    Arc::new(Mutex::new(HashMap::new()))
}

pub fn get_local_ip() -> String {
    let socket = UdpSocket::bind("0.0.0.0:0").expect("Failed to bind UDP socket");
    socket
        .connect("8.8.8.8:80")
        .expect("Failed to connect to determine local IP");
    socket
        .local_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn get_subnet_broadcast() -> Ipv4Addr {
    let local_ip = get_local_ip();
    if let Ok(ip) = local_ip.parse::<Ipv4Addr>() {
        let octets = ip.octets();
        // Assume /24 subnet (most home networks)
        Ipv4Addr::new(octets[0], octets[1], octets[2], 255)
    } else {
        Ipv4Addr::BROADCAST
    }
}

pub fn start_broadcast(
    discovery_port: u16,
    hostname: String,
    stream_port: u16,
    role: Arc<Mutex<Role>>,
    running: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let socket = match UdpSocket::bind("0.0.0.0:0") {
            Ok(s) => s,
            Err(e) => {
                warn!("Discovery broadcast: failed to bind socket: {}", e);
                return;
            }
        };
        if let Err(e) = socket.set_broadcast(true) {
            warn!("Discovery broadcast: failed to set broadcast: {}", e);
            return;
        }

        info!("Discovery broadcast started on port {}", discovery_port);

        while running.load(std::sync::atomic::Ordering::Relaxed) {
            let local_ip = get_local_ip();
            let current_role = {
                let r = role.lock().unwrap();
                format!("{}", *r)
            };

            let packet = DiscoveryPacket {
                hostname: hostname.clone(),
                ip: local_ip,
                role: current_role,
                version: crate::protocol::PROTOCOL_VERSION,
                stream_port,
            };

            if let Ok(data) = serde_json::to_vec(&packet) {
                // Send to both 255.255.255.255 and subnet broadcast for compatibility
                let global = SocketAddr::new(
                    std::net::IpAddr::V4(Ipv4Addr::BROADCAST),
                    discovery_port,
                );
                let subnet = SocketAddr::new(
                    std::net::IpAddr::V4(get_subnet_broadcast()),
                    discovery_port,
                );
                let _ = socket.send_to(&data, global);
                if subnet != global {
                    let _ = socket.send_to(&data, subnet);
                }
            }

            std::thread::sleep(BROADCAST_INTERVAL);
        }

        info!("Discovery broadcast stopped");
    })
}

pub fn start_listener(
    discovery_port: u16,
    peers: PeerMap,
    running: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let addr = SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            discovery_port,
        );

        // Use socket2 for SO_REUSEADDR before bind
        let socket = match (|| -> std::io::Result<UdpSocket> {
            let sock = socket2::Socket::new(
                socket2::Domain::IPV4,
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )?;
            sock.set_reuse_address(true)?;
            #[cfg(target_os = "macos")]
            sock.set_reuse_port(true)?;
            sock.set_broadcast(true)?;
            sock.bind(&addr.into())?;
            Ok(sock.into())
        })() {
            Ok(s) => s,
            Err(e) => {
                warn!("Discovery listener: failed to bind on port {}: {}", discovery_port, e);
                return;
            }
        };
        if let Err(e) = socket.set_read_timeout(Some(Duration::from_secs(1))) {
            warn!("Discovery listener: failed to set read timeout: {}", e);
        }

        info!("Discovery listener started on port {}", discovery_port);
        let local_ip = get_local_ip();
        let mut buf = [0u8; 4096];

        while running.load(std::sync::atomic::Ordering::Relaxed) {
            match socket.recv_from(&mut buf) {
                Ok((len, _src_addr)) => {
                    if let Ok(packet) = serde_json::from_slice::<DiscoveryPacket>(&buf[..len]) {
                        if packet.ip == local_ip {
                            continue;
                        }

                        debug!(
                            "Discovered peer: {} at {} (role: {})",
                            packet.hostname, packet.ip, packet.role
                        );

                        let mut peers = peers.lock().unwrap();

                        let is_new = !peers.contains_key(&packet.ip);
                        peers.insert(
                            packet.ip.clone(),
                            PeerInfo {
                                hostname: packet.hostname.clone(),
                                ip: packet.ip.clone(),
                                role: packet.role.clone(),
                                stream_port: packet.stream_port,
                                last_seen: Instant::now(),
                            },
                        );

                        if is_new {
                            info!(
                                "New peer found: {} ({}) - role: {}",
                                packet.hostname, packet.ip, packet.role
                            );
                        }

                        // Prune stale peers
                        peers.retain(|_, p| p.last_seen.elapsed() < PEER_TIMEOUT);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Timeout, check for stale peers
                    let mut peers = peers.lock().unwrap();
                    peers.retain(|_, p| p.last_seen.elapsed() < PEER_TIMEOUT);
                }
                Err(e) => {
                    debug!("Discovery listener recv error: {}", e);
                }
            }
        }

        info!("Discovery listener stopped");
    })
}

pub fn find_peer(peers: &PeerMap) -> Option<PeerInfo> {
    let peers = peers.lock().unwrap();
    // Prefer peers advertising as "primary"
    if let Some(p) = peers.values().find(|p| p.role == "primary") {
        return Some(p.clone());
    }
    peers.values().next().cloned()
}
