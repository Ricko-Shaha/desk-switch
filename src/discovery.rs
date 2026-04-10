use log::{debug, info, warn};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::{DiscoveryPacket, Role};

const BROADCAST_INTERVAL: Duration = Duration::from_secs(2);
const PEER_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Detect the local LAN IP, preferring WiFi/ethernet over VPN/tunnel interfaces.
/// Tries multiple targets on different subnets to find a non-VPN route.
pub fn get_local_ip() -> String {
    // Try LAN-local targets first (common router IPs), then fallback to internet
    // This helps avoid picking VPN interfaces that route to the internet
    let targets = [
        "192.168.0.1:80",
        "192.168.1.1:80",
        "10.0.0.1:80",
        "8.8.8.8:80",
    ];

    let mut best_ip: Option<String> = None;

    for target in &targets {
        if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
            if socket.connect(target).is_ok() {
                if let Ok(addr) = socket.local_addr() {
                    let ip = addr.ip().to_string();
                    // Prefer 192.168.x.x or 10.x.x.x over 172.16-31.x.x (often VPN)
                    if ip.starts_with("192.168.") || ip.starts_with("10.") {
                        return ip;
                    }
                    if best_ip.is_none() {
                        best_ip = Some(ip);
                    }
                }
            }
        }
    }

    best_ip.unwrap_or_else(|| "127.0.0.1".to_string())
}

fn get_subnet_broadcast() -> Ipv4Addr {
    let local_ip = get_local_ip();
    if let Ok(ip) = local_ip.parse::<Ipv4Addr>() {
        let octets = ip.octets();
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
                Ok((len, src_addr)) => {
                    if let Ok(packet) = serde_json::from_slice::<DiscoveryPacket>(&buf[..len]) {
                        // Use the ACTUAL source IP from the UDP packet, not the self-reported IP.
                        // The self-reported IP may be a VPN/tunnel address that we can't reach.
                        let actual_ip = src_addr.ip().to_string();

                        if actual_ip == local_ip || packet.ip == local_ip {
                            continue;
                        }

                        let connect_ip = if actual_ip.starts_with("192.168.")
                            || actual_ip.starts_with("10.")
                        {
                            actual_ip.clone()
                        } else if packet.ip.starts_with("192.168.")
                            || packet.ip.starts_with("10.")
                        {
                            packet.ip.clone()
                        } else {
                            actual_ip.clone()
                        };

                        debug!(
                            "Discovered peer: {} (reported: {}, actual: {}, using: {}, role: {})",
                            packet.hostname, packet.ip, actual_ip, connect_ip, packet.role
                        );

                        let mut peers = peers.lock().unwrap();

                        let is_new = !peers.contains_key(&connect_ip);
                        peers.insert(
                            connect_ip.clone(),
                            PeerInfo {
                                hostname: packet.hostname.clone(),
                                ip: connect_ip.clone(),
                                role: packet.role.clone(),
                                stream_port: packet.stream_port,
                                last_seen: Instant::now(),
                            },
                        );

                        if is_new {
                            info!(
                                "New peer: {} at {} (actual src: {}, role: {})",
                                packet.hostname, connect_ip, actual_ip, packet.role
                            );
                        }

                        peers.retain(|_, p| p.last_seen.elapsed() < PEER_TIMEOUT);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
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
    if let Some(p) = peers.values().find(|p| p.role == "primary") {
        return Some(p.clone());
    }
    peers.values().next().cloned()
}
