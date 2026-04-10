use anyhow::{anyhow, Result};
use log::{info, warn};
use scrap::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};

use crate::capture::CaptureSession;
use crate::config::Config;
use crate::discovery::{self, PeerInfo, PeerMap};
use crate::input::{InputCaptureSession, InputSimulator};
use crate::protocol::{self, Message, Role};
use crate::viewer::ViewerSession;

fn get_primary_screen_size() -> (u32, u32) {
    Display::all()
        .ok()
        .and_then(|d| d.into_iter().next())
        .map(|d| (d.width() as u32, d.height() as u32))
        .unwrap_or((1920, 1080))
}

pub struct Service {
    config: Config,
    role: Arc<Mutex<Role>>,
    running: Arc<AtomicBool>,
    peers: PeerMap,
}

impl Service {
    pub fn new(config: Config) -> Self {
        let default_role = match config.default_role.as_str() {
            "primary" => Role::Primary,
            "display" => Role::Display,
            _ => Role::Idle,
        };

        Self {
            config,
            role: Arc::new(Mutex::new(default_role)),
            running: Arc::new(AtomicBool::new(true)),
            peers: discovery::new_peer_map(),
        }
    }

    pub async fn run(&self) -> Result<()> {
        info!("Desk Switch service starting...");
        info!("Hostname: {}", self.config.hostname);
        info!("Default role: {}", self.config.default_role);
        info!(
            "Stream port: {}, Discovery port: {}",
            self.config.stream_port, self.config.discovery_port
        );

        // Start discovery
        let broadcast_handle = discovery::start_broadcast(
            self.config.discovery_port,
            self.config.hostname.clone(),
            self.config.stream_port,
            self.role.clone(),
            self.running.clone(),
        );

        let listener_handle = discovery::start_listener(
            self.config.discovery_port,
            self.peers.clone(),
            self.running.clone(),
        );

        // Setup ctrl-c handler
        let running = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Shutting down...");
            running.store(false, Ordering::Relaxed);
        })
        .expect("Failed to set Ctrl-C handler");

        // Main role loop
        while self.running.load(Ordering::Relaxed) {
            let current_role = { *self.role.lock().unwrap() };

            match current_role {
                Role::Idle => {
                    self.run_idle().await?;
                }
                Role::Primary => {
                    if let Err(e) = self.run_primary().await {
                        warn!("Primary mode error: {}. Returning to idle.", e);
                        *self.role.lock().unwrap() = Role::Idle;
                    }
                }
                Role::Display => {
                    if let Err(e) = self.run_display().await {
                        warn!("Display mode error: {}. Returning to idle.", e);
                        *self.role.lock().unwrap() = Role::Idle;
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        info!("Service shutting down...");
        self.running.store(false, Ordering::Relaxed);
        let _ = broadcast_handle.join();
        let _ = listener_handle.join();
        info!("Service stopped.");
        Ok(())
    }

    async fn run_idle(&self) -> Result<()> {
        // Wait in idle until role changes
        while self.running.load(Ordering::Relaxed) {
            {
                let role = self.role.lock().unwrap();
                if *role != Role::Idle {
                    break;
                }
            }

            // Log peers periodically
            if let Some(peer) = discovery::find_peer(&self.peers) {
                info!(
                    "Peer available: {} ({}) - role: {}",
                    peer.hostname, peer.ip, peer.role
                );
            }

            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        Ok(())
    }

    async fn run_primary(&self) -> Result<()> {
        info!("=== PRIMARY MODE ===");

        #[cfg(target_os = "macos")]
        {
            if !crate::platform::macos::has_screen_recording_permission() {
                warn!("Screen Recording permission not granted — requesting...");
                crate::platform::macos::ensure_screen_recording();
                return Err(anyhow!(
                    "Screen Recording permission required. Grant it in System Preferences → Privacy & Security → Screen Recording, then restart."
                ));
            }
        }

        info!(
            "Capturing monitor {} at quality {} (max {} FPS)",
            self.config.capture_monitor, self.config.capture_quality, self.config.max_fps
        );

        let mut capture = CaptureSession::start(
            self.config.capture_monitor,
            self.config.capture_quality,
            self.config.max_fps,
        )?;

        // Listen for incoming connections from display machines
        let listener =
            TcpListener::bind(format!("0.0.0.0:{}", self.config.stream_port)).await?;
        info!("Listening for display connections on port {}", self.config.stream_port);

        while self.running.load(Ordering::Relaxed) {
            let role_check = { *self.role.lock().unwrap() };
            if role_check != Role::Primary {
                info!("Role changed from Primary, stopping capture...");
                break;
            }

            // Accept connection with timeout
            let accept_result = tokio::time::timeout(
                Duration::from_secs(2),
                listener.accept(),
            )
            .await;

            match accept_result {
                Ok(Ok((stream, addr))) => {
                    info!("Display connected from {}", addr);
                    self.handle_primary_connection(
                        stream,
                        &capture,
                    )
                    .await?;
                }
                Ok(Err(e)) => {
                    warn!("Accept error: {}", e);
                }
                Err(_) => {
                    // Timeout, loop back to check role
                    continue;
                }
            }
        }

        capture.stop();
        Ok(())
    }

    async fn handle_primary_connection(
        &self,
        stream: TcpStream,
        capture: &CaptureSession,
    ) -> Result<()> {
        let std_stream = stream.into_std()?;
        std_stream.set_nonblocking(false)?;
        let std_stream2 = std_stream.try_clone()?;

        let mut writer = std::io::BufWriter::new(std_stream);
        let mut reader = std::io::BufReader::new(std_stream2);

        // Handshake
        let handshake_msg = protocol::read_message_sync(&mut reader)?;
        match handshake_msg {
            Message::Handshake {
                version,
                auth_hash,
                hostname,
            } => {
                if auth_hash != self.config.auth_hash() {
                    warn!("Authentication failed from {}", hostname);
                    let reject = Message::HandshakeAck { accepted: false };
                    protocol::write_message_sync(&mut writer, &reject)?;
                    return Err(anyhow!("Authentication failed"));
                }
                info!("Authenticated display: {} (v{})", hostname, version);
                let accept = Message::HandshakeAck { accepted: true };
                protocol::write_message_sync(&mut writer, &accept)?;
            }
            _ => return Err(anyhow!("Expected Handshake, got {:?}", handshake_msg)),
        }

        let (screen_w, screen_h) = get_primary_screen_size();
        let running = self.running.clone();
        let role = self.role.clone();
        let input_handle = std::thread::spawn(move || {
            let mut input_sim = InputSimulator::new(screen_w, screen_h);

            loop {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                let role_check = { *role.lock().unwrap() };
                if role_check != Role::Primary {
                    break;
                }

                match protocol::read_message_sync(&mut reader) {
                    Ok(msg) => match &msg {
                        Message::MouseMove { .. }
                        | Message::MouseClick { .. }
                        | Message::MouseScroll { .. }
                        | Message::KeyEvent { .. } => {
                            input_sim.handle_message(&msg);
                        }
                        Message::RoleSwitch { new_role } => {
                            info!("Received role switch request: {}", new_role);
                            let mut r = role.lock().unwrap();
                            *r = if *new_role == 2 {
                                Role::Display
                            } else {
                                Role::Idle
                            };
                            break;
                        }
                        Message::Heartbeat { .. } => {}
                        _ => {}
                    },
                    Err(e) => {
                        warn!("Input read error: {}", e);
                        break;
                    }
                }
            }
        });

        // Stream frames to display
        let frame_rx = capture.frame_rx.clone();
        while self.running.load(Ordering::Relaxed) {
            let role_check = { *self.role.lock().unwrap() };
            if role_check != Role::Primary {
                break;
            }

            match frame_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(frame) => {
                    let msg = Message::Frame {
                        width: frame.width,
                        height: frame.height,
                        jpeg_data: frame.jpeg_data,
                    };
                    if let Err(e) = protocol::write_message_sync(&mut writer, &msg) {
                        warn!("Frame send error: {}", e);
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }

        let _ = input_handle.join();
        info!("Display disconnected");
        Ok(())
    }

    async fn run_display(&self) -> Result<()> {
        info!("=== DISPLAY MODE ===");

        // Wait for a peer to connect to
        let peer = loop {
            if !self.running.load(Ordering::Relaxed) {
                return Ok(());
            }
            let role_check = { *self.role.lock().unwrap() };
            if role_check != Role::Display {
                return Ok(());
            }

            if let Some(peer) = discovery::find_peer(&self.peers) {
                break peer;
            }

            info!("Waiting for primary peer...");
            tokio::time::sleep(Duration::from_secs(2)).await;
        };

        info!(
            "Connecting to primary: {} ({}:{})",
            peer.hostname, peer.ip, peer.stream_port
        );

        let stream = TcpStream::connect(format!("{}:{}", peer.ip, peer.stream_port)).await?;
        let std_stream = stream.into_std()?;
        std_stream.set_nonblocking(false)?;
        let std_stream2 = std_stream.try_clone()?;

        let mut writer = std::io::BufWriter::new(std_stream);
        let mut reader = std::io::BufReader::new(std_stream2);

        // Send handshake
        let handshake = Message::Handshake {
            version: protocol::PROTOCOL_VERSION,
            auth_hash: self.config.auth_hash(),
            hostname: self.config.hostname.clone(),
        };
        protocol::write_message_sync(&mut writer, &handshake)?;

        // Read handshake response
        let response = protocol::read_message_sync(&mut reader)?;
        match response {
            Message::HandshakeAck { accepted: true } => {
                info!("Authenticated with primary");
            }
            Message::HandshakeAck { accepted: false } => {
                return Err(anyhow!("Authentication rejected by primary"));
            }
            _ => return Err(anyhow!("Unexpected handshake response")),
        }

        // Start viewer
        let viewer_running = Arc::new(AtomicBool::new(true));
        let mut viewer = ViewerSession::start(viewer_running.clone());

        // Start input capture with display machine's screen dimensions for coordinate normalization
        let input_running = Arc::new(AtomicBool::new(true));
        let (screen_w, screen_h) = get_primary_screen_size();
        let mut input_capture =
            InputCaptureSession::start(input_running.clone(), screen_w as f64, screen_h as f64);

        // Input sender thread (std thread because writer is blocking)
        let running_input = self.running.clone();
        let role_input = self.role.clone();
        let event_rx = input_capture.event_rx.clone();
        let input_sender = std::thread::spawn(move || {
            while running_input.load(Ordering::Relaxed) {
                let role_check = { *role_input.lock().unwrap() };
                if role_check != Role::Display {
                    break;
                }

                match event_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(msg) => {
                        if let Err(e) = protocol::write_message_sync(&mut writer, &msg) {
                            warn!("Input send error: {}", e);
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        let hb = Message::Heartbeat {
                            timestamp_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                        };
                        let _ = protocol::write_message_sync(&mut writer, &hb);
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        // Receive frames and feed to viewer (blocking read in current context)
        let frame_receiver = {
            let running = self.running.clone();
            let role = self.role.clone();
            let jpeg_tx = viewer.jpeg_tx.clone();
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    let role_check = { *role.lock().unwrap() };
                    if role_check != Role::Display {
                        break;
                    }

                    match protocol::read_message_sync(&mut reader) {
                        Ok(Message::Frame {
                            width,
                            height,
                            jpeg_data,
                        }) => {
                            let _ = jpeg_tx.try_send((width, height, jpeg_data));
                        }
                        Ok(Message::RoleSwitch { new_role }) => {
                            info!("Received role switch: {}", new_role);
                            let mut r = role.lock().unwrap();
                            *r = if new_role == 1 {
                                Role::Primary
                            } else {
                                Role::Idle
                            };
                            break;
                        }
                        Ok(Message::Heartbeat { .. }) => {}
                        Ok(other) => {
                            warn!("Unexpected message in display mode: {:?}", other);
                        }
                        Err(e) => {
                            warn!("Frame receive error: {}", e);
                            break;
                        }
                    }
                }
            })
        };

        // Wait for either the viewer to close or network threads to finish
        while self.running.load(Ordering::Relaxed) && viewer.is_running() {
            let role_check = { *self.role.lock().unwrap() };
            if role_check != Role::Display {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        viewer_running.store(false, Ordering::Relaxed);
        input_running.store(false, Ordering::Relaxed);
        let _ = input_sender.join();
        let _ = frame_receiver.join();
        input_capture.stop();
        viewer.stop();
        info!("Display mode ended");
        Ok(())
    }

    #[allow(dead_code)]
    pub fn switch_role(&self) {
        let mut role = self.role.lock().unwrap();
        let new_role = match *role {
            Role::Primary => Role::Display,
            Role::Display => Role::Primary,
            Role::Idle => Role::Primary,
        };
        info!("Switching role: {} -> {}", *role, new_role);
        *role = new_role;
    }

    #[allow(dead_code)]
    pub fn set_role(&self, new_role: Role) {
        let mut role = self.role.lock().unwrap();
        info!("Setting role: {} -> {}", *role, new_role);
        *role = new_role;
    }

    #[allow(dead_code)]
    pub fn current_role(&self) -> Role {
        *self.role.lock().unwrap()
    }

    #[allow(dead_code)]
    pub fn peer_info(&self) -> Option<PeerInfo> {
        discovery::find_peer(&self.peers)
    }
}
