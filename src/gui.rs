use crate::capture::{self, CaptureSession};
use crate::config::{load_config, save_config, Config};
use crate::discovery::{self, PeerMap};
use crate::protocol::{self, Message, Role, PROTOCOL_VERSION};
use crate::virtual_display::VirtualMonitor;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui::{
    self, Align2, Button, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense,
    Stroke, TextureHandle, TextureOptions, Vec2,
};
use log::info;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// ── Color Palette ───────────────────────────────────────────────────────────

const BG: Color32 = Color32::from_rgb(13, 13, 28);
const SURFACE: Color32 = Color32::from_rgb(20, 20, 44);
const SURFACE_LIGHT: Color32 = Color32::from_rgb(28, 28, 56);
const BORDER: Color32 = Color32::from_rgb(40, 40, 72);
const ACCENT_BLUE: Color32 = Color32::from_rgb(67, 97, 238);
const ACCENT_BLUE_HOVER: Color32 = Color32::from_rgb(87, 117, 255);
const ACCENT_PURPLE: Color32 = Color32::from_rgb(139, 92, 246);
const ACCENT_PURPLE_HOVER: Color32 = Color32::from_rgb(159, 112, 255);
const RED: Color32 = Color32::from_rgb(239, 68, 68);
const RED_HOVER: Color32 = Color32::from_rgb(255, 88, 88);
const GREEN: Color32 = Color32::from_rgb(16, 185, 129);
const YELLOW: Color32 = Color32::from_rgb(245, 158, 11);
const TEXT: Color32 = Color32::from_rgb(243, 244, 246);
const TEXT_DIM: Color32 = Color32::from_rgb(107, 114, 128);
const TEXT_BLUE: Color32 = Color32::from_rgb(147, 197, 253);

// ── Application State ───────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum AppMode {
    Idle,
    Primary,
    Display,
}

struct LogEntry {
    msg: String,
}

pub struct DeskSwitchApp {
    config: Config,
    mode: AppMode,
    first_launch: bool,

    // Shared state
    running: Arc<AtomicBool>,
    peers: PeerMap,

    // Discovery threads
    discovery_running: bool,
    discovery_role: Arc<Mutex<Role>>,

    // Primary mode
    virtual_monitor: Option<VirtualMonitor>,
    capture_session: Option<CaptureSession>,
    primary_handle: Option<thread::JoinHandle<()>>,
    primary_fps: Arc<Mutex<u32>>,
    primary_peer_name: Arc<Mutex<Option<String>>>,

    // Display mode
    display_handle: Option<thread::JoinHandle<()>>,
    pixel_rx: Option<Receiver<DecodedFrame>>,
    input_tx: Option<Sender<Message>>,
    frame_texture: Option<TextureHandle>,
    display_fps: Arc<Mutex<u32>>,
    display_connected: Arc<AtomicBool>,

    // Logs
    logs: Vec<LogEntry>,
    log_rx: Receiver<String>,
    log_tx: Sender<String>,

    // UI state
    show_settings: bool,
    auth_key_copied: Option<Instant>,
    manual_ip: String,
}

struct DecodedFrame {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn launch_gui() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([680.0, 720.0])
            .with_min_inner_size([480.0, 520.0])
            .with_title("Desk Switch"),
        ..Default::default()
    };

    eframe::run_native(
        "Desk Switch",
        options,
        Box::new(|cc| Ok(Box::new(DeskSwitchApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {}", e))
}

// ── Construction ────────────────────────────────────────────────────────────

impl DeskSwitchApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);

        let (config, first_launch) = match load_config() {
            Ok(c) => (c, false),
            Err(_) => {
                let c = Config::default();
                let _ = save_config(&c);
                (c, true)
            }
        };

        // Always ensure firewall exception on macOS (can get revoked after app updates)
        #[cfg(target_os = "macos")]
        {
            std::thread::spawn(|| {
                crate::platform::macos::add_firewall_exception();
            });
        }

        let (log_tx, log_rx) = bounded::<String>(256);

        Self {
            config,
            mode: AppMode::Idle,
            first_launch,
            running: Arc::new(AtomicBool::new(true)),
            peers: discovery::new_peer_map(),
            discovery_running: false,
            discovery_role: Arc::new(Mutex::new(Role::Idle)),
            virtual_monitor: None,
            capture_session: None,
            primary_handle: None,
            primary_fps: Arc::new(Mutex::new(0)),
            primary_peer_name: Arc::new(Mutex::new(None)),
            display_handle: None,
            pixel_rx: None,
            input_tx: None,
            frame_texture: None,
            display_fps: Arc::new(Mutex::new(0)),
            display_connected: Arc::new(AtomicBool::new(false)),
            logs: Vec::new(),
            log_rx,
            log_tx,
            show_settings: false,
            auth_key_copied: None,
            manual_ip: String::new(),
        }
    }

    fn push_log(&mut self, msg: impl Into<String>) {
        let s = msg.into();
        info!("{}", s);
        self.logs.push(LogEntry { msg: s });
        if self.logs.len() > 200 {
            self.logs.remove(0);
        }
    }

    fn drain_logs(&mut self) {
        while let Ok(msg) = self.log_rx.try_recv() {
            self.logs.push(LogEntry { msg });
        }
        if self.logs.len() > 200 {
            self.logs.drain(0..self.logs.len() - 200);
        }
    }

    // ── Discovery ───────────────────────────────────────────────────────

    fn ensure_discovery(&mut self) {
        if self.discovery_running {
            return;
        }
        let running = self.running.clone();
        discovery::start_broadcast(
            self.config.discovery_port,
            self.config.hostname.clone(),
            self.config.stream_port,
            self.discovery_role.clone(),
            running.clone(),
        );
        discovery::start_listener(self.config.discovery_port, self.peers.clone(), running);
        self.discovery_running = true;
        self.push_log("Discovery started");
    }

    fn set_discovery_role(&self, role: Role) {
        *self.discovery_role.lock().unwrap() = role;
    }

    fn peer_display_name(&self) -> Option<String> {
        discovery::find_peer(&self.peers).map(|p| format!("{} ({})", p.hostname, p.ip))
    }

    // ── Start Primary ───────────────────────────────────────────────────

    fn start_primary(&mut self) {
        self.ensure_discovery();
        self.mode = AppMode::Primary;
        self.set_discovery_role(Role::Primary);

        let capture_monitor = if self.config.use_virtual_display {
            self.push_log(format!(
                "Creating virtual display ({}x{})...",
                self.config.virtual_display_width, self.config.virtual_display_height
            ));

            match VirtualMonitor::create(
                self.config.virtual_display_width,
                self.config.virtual_display_height,
                self.config.max_fps,
            ) {
                Ok(vm) => {
                    let idx = vm.display_index;
                    self.push_log(format!(
                        "Virtual display created → capturing display {}",
                        idx
                    ));
                    self.virtual_monitor = Some(vm);
                    idx
                }
                Err(e) => {
                    self.push_log(format!(
                        "Virtual display unavailable ({}). Falling back to mirror mode — capturing display {}",
                        e, self.config.capture_monitor
                    ));
                    self.config.capture_monitor
                }
            }
        } else {
            self.push_log(format!(
                "Starting PRIMARY (mirror) mode — capturing display {}",
                self.config.capture_monitor
            ));
            self.config.capture_monitor
        };

        let capture = match CaptureSession::start(
            capture_monitor,
            self.config.capture_quality,
            self.config.max_fps,
        ) {
            Ok(c) => c,
            Err(e) => {
                self.push_log(format!("Capture error: {}", e));
                self.virtual_monitor = None;
                self.mode = AppMode::Idle;
                return;
            }
        };

        let frame_rx = capture.frame_rx.clone();
        let port = self.config.stream_port;
        let auth_hash = self.config.auth_hash();
        let running = self.running.clone();
        let fps_counter = self.primary_fps.clone();
        let peer_name = self.primary_peer_name.clone();
        let log_tx = self.log_tx.clone();

        let handle = thread::spawn(move || {
            primary_network_loop(frame_rx, port, auth_hash, running, fps_counter, peer_name, log_tx);
        });

        self.capture_session = Some(capture);
        self.primary_handle = Some(handle);
    }

    // ── Start Display ───────────────────────────────────────────────────

    fn start_display(&mut self) {
        self.ensure_discovery();
        self.mode = AppMode::Display;
        self.set_discovery_role(Role::Display);
        self.push_log("Starting DISPLAY mode — searching for primary...");

        let (pixel_tx, pixel_rx) = bounded::<DecodedFrame>(2);
        let (input_tx, input_rx) = bounded::<Message>(64);

        let peers = self.peers.clone();
        let auth_hash = self.config.auth_hash();
        let hostname = self.config.hostname.clone();
        let running = self.running.clone();
        let fps_counter = self.display_fps.clone();
        let connected = self.display_connected.clone();
        let log_tx = self.log_tx.clone();

        let handle = thread::spawn(move || {
            display_network_loop(
                peers, auth_hash, hostname, running, pixel_tx, input_rx, fps_counter, connected,
                log_tx,
            );
        });

        self.pixel_rx = Some(pixel_rx);
        self.input_tx = Some(input_tx);
        self.display_handle = Some(handle);
    }

    // ── Stop ────────────────────────────────────────────────────────────

    fn stop(&mut self) {
        self.push_log("Stopping...");
        self.set_discovery_role(Role::Idle);
        self.running.store(false, Ordering::Relaxed);

        if let Some(mut cap) = self.capture_session.take() {
            cap.stop();
        }
        if let Some(h) = self.primary_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.display_handle.take() {
            let _ = h.join();
        }

        if let Some(mut vm) = self.virtual_monitor.take() {
            self.push_log("Removing virtual display...");
            let _ = vm.destroy();
        }

        self.pixel_rx = None;
        self.input_tx = None;
        self.frame_texture = None;
        self.display_connected.store(false, Ordering::Relaxed);
        *self.primary_peer_name.lock().unwrap() = None;
        *self.primary_fps.lock().unwrap() = 0;
        *self.display_fps.lock().unwrap() = 0;

        self.running = Arc::new(AtomicBool::new(true));
        self.discovery_running = false;
        self.mode = AppMode::Idle;
        self.push_log("Stopped");
    }
}

// ── UI Rendering ────────────────────────────────────────────────────────────

impl eframe::App for DeskSwitchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_logs();

        if self.mode == AppMode::Display {
            if let Some(rx) = &self.pixel_rx {
                let mut latest = None;
                while let Ok(f) = rx.try_recv() {
                    latest = Some(f);
                }
                if let Some(f) = latest {
                    let img = egui::ColorImage::new([f.width, f.height], f.pixels);
                    self.frame_texture =
                        Some(ctx.load_texture("remote", img, TextureOptions::LINEAR));
                }
            }

            self.forward_input(&ctx);
        }

        Frame::new()
            .fill(BG)
            .inner_margin(Margin::ZERO)
            .show(ui, |ui| match self.mode {
                AppMode::Idle => self.draw_idle(ui),
                AppMode::Primary => self.draw_primary(ui),
                AppMode::Display => self.draw_display(ui),
            });

        if self.mode != AppMode::Idle {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}

impl DeskSwitchApp {
    // ── Idle Screen ─────────────────────────────────────────────────────

    fn draw_idle(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_min_width(avail.x);
            ui.add_space(32.0);

            // Title
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("DESK SWITCH").font(FontId::proportional(32.0)).color(TEXT));
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Cross-platform KVM switch")
                        .font(FontId::proportional(14.0))
                        .color(TEXT_DIM),
                );
            });

            ui.add_space(28.0);

            // First-launch banner
            if self.first_launch {
                ui.vertical_centered(|ui| {
                    card(ui, Color32::from_rgb(30, 25, 50), |ui| {
                        ui.label(
                            RichText::new("Welcome to Desk Switch!")
                                .color(TEXT)
                                .font(FontId::proportional(16.0)),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Your auth key (share with the other machine):")
                                .color(TEXT_BLUE)
                                .font(FontId::proportional(13.0)),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let key_short = if self.config.auth_key.len() > 24 {
                                format!("{}...", &self.config.auth_key[..24])
                            } else {
                                self.config.auth_key.clone()
                            };
                            ui.label(
                                RichText::new(key_short)
                                    .font(FontId::monospace(13.0))
                                    .color(TEXT),
                            );
                            if ui
                                .add(Button::new(
                                    RichText::new("Copy Key")
                                        .color(ACCENT_BLUE)
                                        .font(FontId::proportional(12.0)),
                                ))
                                .clicked()
                            {
                                ui.ctx().copy_text(self.config.auth_key.clone());
                                self.auth_key_copied = Some(Instant::now());
                            }
                            if self.auth_key_copied.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                                ui.label(
                                    RichText::new("Copied!")
                                        .color(GREEN)
                                        .font(FontId::proportional(12.0)),
                                );
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(
                                "On the other machine, run the installer, then paste\nthis key in Settings below.",
                            )
                            .color(YELLOW)
                            .font(FontId::proportional(12.0)),
                        );
                    });
                });
                ui.add_space(16.0);
            }

            // Mode selection cards
            ui.horizontal(|ui| {
                let card_width = (avail.x - 64.0) / 2.0;

                ui.add_space(16.0);
                let primary_clicked = mode_card(
                    ui,
                    card_width,
                    "PRIMARY",
                    "Stream your screen to the\nother laptop",
                    ACCENT_BLUE,
                    ACCENT_BLUE_HOVER,
                    "Be the main workstation",
                );
                ui.add_space(8.0);
                let display_clicked = mode_card(
                    ui,
                    card_width,
                    "DISPLAY",
                    "Show the other laptop's\nscreen here",
                    ACCENT_PURPLE,
                    ACCENT_PURPLE_HOVER,
                    "Be an extra screen",
                );
                ui.add_space(16.0);

                if primary_clicked {
                    self.start_primary();
                }
                if display_clicked {
                    self.start_display();
                }
            });

            ui.add_space(24.0);

            // Peer status
            ui.vertical_centered(|ui| {
                card(ui, SURFACE, |ui| {
                    ui.horizontal(|ui| {
                        let peer = self.peer_display_name();
                        let (dot, label) = if let Some(ref name) = peer {
                            (GREEN, format!("Peer found: {}", name))
                        } else {
                            (TEXT_DIM, "Searching for peer on network...".to_string())
                        };
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter()
                            .circle_filled(rect.center(), 5.0, dot);
                        ui.label(RichText::new(label).color(TEXT).font(FontId::proportional(13.0)));
                    });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Can't find peer? Enter IP manually:")
                            .color(TEXT_DIM)
                            .font(FontId::proportional(12.0)),
                    );
                    ui.horizontal(|ui| {
                        let te = egui::TextEdit::singleline(&mut self.manual_ip)
                            .hint_text("e.g. 192.168.1.42")
                            .font(FontId::monospace(13.0))
                            .desired_width(180.0);
                        ui.add(te);
                        if ui
                            .add(Button::new(
                                RichText::new("Connect")
                                    .color(Color32::WHITE)
                                    .font(FontId::proportional(13.0)),
                            ))
                            .clicked()
                            && !self.manual_ip.trim().is_empty()
                        {
                            let ip = self.manual_ip.trim().to_string();
                            let port = self.config.stream_port;
                            let mut peers = self.peers.lock().unwrap();
                            peers.insert(
                                ip.clone(),
                                discovery::PeerInfo {
                                    hostname: format!("manual ({})", ip),
                                    ip: ip.clone(),
                                    role: "primary".to_string(),
                                    stream_port: port,
                                    last_seen: Instant::now(),
                                },
                            );
                            let _ = self.log_tx.try_send(format!("Manually added peer: {}", ip));
                        }
                    });
                });
            });

            ui.add_space(20.0);

            // Settings toggle
            ui.vertical_centered(|ui| {
                if ui
                    .add(
                        Button::new(
                            RichText::new(if self.show_settings {
                                "Hide Settings"
                            } else {
                                "Settings"
                            })
                            .color(TEXT_BLUE)
                            .font(FontId::proportional(13.0)),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.show_settings = !self.show_settings;
                }
            });

            if self.show_settings {
                ui.add_space(8.0);
                self.draw_settings(ui);
            }

            ui.add_space(16.0);

            // Logs
            if !self.logs.is_empty() {
                self.draw_logs(ui);
            }

            ui.add_space(24.0);
        });
    }

    // ── Primary Screen ──────────────────────────────────────────────────

    fn draw_primary(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);

        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("PRIMARY MODE")
                    .font(FontId::proportional(24.0))
                    .color(ACCENT_BLUE),
            );
            ui.add_space(4.0);
            let mode_label = if self.virtual_monitor.is_some() {
                "Extended display (3rd screen)"
            } else {
                "Mirror mode (showing existing screen)"
            };
            let mode_color = if self.virtual_monitor.is_some() {
                GREEN
            } else {
                YELLOW
            };
            ui.label(
                RichText::new(mode_label)
                    .font(FontId::proportional(14.0))
                    .color(mode_color),
            );
        });

        ui.add_space(24.0);

        // Status card
        ui.vertical_centered(|ui| {
            card(ui, SURFACE, |ui| {
                let peer_name = self.primary_peer_name.lock().unwrap().clone();
                let fps = *self.primary_fps.lock().unwrap();

                ui.horizontal(|ui| {
                    let (dot_color, status) = if peer_name.is_some() {
                        (GREEN, "Connected")
                    } else {
                        (YELLOW, "Waiting for display...")
                    };
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 5.0, dot_color);
                    ui.label(RichText::new(status).color(TEXT).font(FontId::proportional(15.0)));
                });

                if let Some(ref name) = peer_name {
                    ui.add_space(8.0);
                    stat_row(ui, "Peer", name);
                }

                ui.add_space(4.0);
                stat_row(ui, "FPS", &fps.to_string());
                stat_row(ui, "Quality", &format!("{}%", self.config.capture_quality));

                let local_ip = discovery::get_local_ip();
                stat_row(
                    ui,
                    "Your IP",
                    &format!("{}:{}", local_ip, self.config.stream_port),
                );
            });
        });

        ui.add_space(24.0);

        // Action buttons
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            if styled_button(ui, "Stop", RED, RED_HOVER, 120.0).clicked() {
                self.stop();
            }
            ui.add_space(12.0);
            if styled_button(ui, "Switch to Display", ACCENT_PURPLE, ACCENT_PURPLE_HOVER, 180.0)
                .clicked()
            {
                self.stop();
                self.start_display();
            }
        });

        ui.add_space(24.0);
        self.draw_logs(ui);
    }

    // ── Display Screen ──────────────────────────────────────────────────

    fn draw_display(&mut self, ui: &mut egui::Ui) {
        let connected = self.display_connected.load(Ordering::Relaxed);

        if let Some(ref tex) = self.frame_texture {
            // Render remote screen filling the window
            let available = ui.available_size();
            let tex_size = tex.size_vec2();

            let scale = (available.x / tex_size.x).min(available.y / tex_size.y);
            let render_size = Vec2::new(tex_size.x * scale, tex_size.y * scale);

            ui.vertical_centered(|ui| {
                ui.add_space(((available.y - render_size.y) / 2.0).max(0.0));
                ui.image(egui::load::SizedTexture::new(tex.id(), render_size));
            });

            // Overlay with status
            let fps = *self.display_fps.lock().unwrap();
            let painter = ui.painter();
            let screen = ui.ctx().content_rect();
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(screen.right() - 200.0, screen.top()),
                    Vec2::new(200.0, 28.0),
                ),
                CornerRadius::ZERO,
                Color32::from_black_alpha(160),
            );
            painter.text(
                egui::pos2(screen.right() - 100.0, screen.top() + 14.0),
                Align2::CENTER_CENTER,
                format!("{} FPS  |  ESC to exit", fps),
                FontId::proportional(12.0),
                TEXT_DIM,
            );
        } else {
            // No frame yet — show waiting UI
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("DISPLAY MODE")
                        .font(FontId::proportional(24.0))
                        .color(ACCENT_PURPLE),
                );
                ui.add_space(12.0);

                let status = if connected {
                    ("Connected — waiting for frames...", GREEN)
                } else {
                    ("Searching for primary...", YELLOW)
                };

                ui.horizontal(|ui| {
                    ui.add_space(ui.available_width() / 2.0 - 120.0);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 5.0, status.1);
                    ui.label(
                        RichText::new(status.0)
                            .color(TEXT)
                            .font(FontId::proportional(14.0)),
                    );
                });
            });

            ui.add_space(32.0);
            ui.vertical_centered(|ui| {
                if styled_button(ui, "Stop", RED, RED_HOVER, 120.0).clicked() {
                    self.stop();
                }
            });

            ui.add_space(24.0);
            self.draw_logs(ui);
        }

        // ESC to stop
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.stop();
        }
    }

    // ── Input Forwarding (Display Mode) ─────────────────────────────────

    fn forward_input(&self, ctx: &egui::Context) {
        let Some(ref tx) = self.input_tx else {
            return;
        };
        if !self.display_connected.load(Ordering::Relaxed) {
            return;
        }

        let screen = ctx.content_rect();

        ctx.input(|input| {
            // Mouse position
            if let Some(pos) = input.pointer.hover_pos() {
                let xr = ((pos.x - screen.left()) / screen.width()).clamp(0.0, 1.0);
                let yr = ((pos.y - screen.top()) / screen.height()).clamp(0.0, 1.0);
                let _ = tx.try_send(Message::MouseMove {
                    x_ratio: xr,
                    y_ratio: yr,
                });
            }

            // Mouse buttons
            if input.pointer.primary_pressed() {
                let _ = tx.try_send(Message::MouseClick {
                    button: 0,
                    pressed: true,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                });
            }
            if input.pointer.primary_released() {
                let _ = tx.try_send(Message::MouseClick {
                    button: 0,
                    pressed: false,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                });
            }
            if input.pointer.secondary_pressed() {
                let _ = tx.try_send(Message::MouseClick {
                    button: 1,
                    pressed: true,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                });
            }
            if input.pointer.secondary_released() {
                let _ = tx.try_send(Message::MouseClick {
                    button: 1,
                    pressed: false,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                });
            }

            // Scroll
            let scroll = input.smooth_scroll_delta;
            if scroll.y.abs() > 0.1 || scroll.x.abs() > 0.1 {
                let _ = tx.try_send(Message::MouseScroll {
                    dx: scroll.x as i32,
                    dy: scroll.y as i32,
                });
            }

            // Keyboard
            for event in &input.events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat: false,
                    ..
                } = event
                {
                    if let Some(code) = egui_key_to_code(key) {
                        let _ = tx.try_send(Message::KeyEvent {
                            key_code: code,
                            pressed: *pressed,
                        });
                    }
                }
            }
        });
    }

    // ── Settings Panel ──────────────────────────────────────────────────

    fn draw_settings(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            card(ui, SURFACE, |ui| {
                ui.label(
                    RichText::new("Settings")
                        .color(TEXT)
                        .font(FontId::proportional(16.0)),
                );
                ui.add_space(12.0);

                let mut quality = self.config.capture_quality as f32;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Quality").color(TEXT_DIM).font(FontId::proportional(13.0)));
                    ui.add_space(16.0);
                    ui.add(egui::Slider::new(&mut quality, 10.0..=100.0).suffix("%"));
                });
                self.config.capture_quality = quality as u8;

                ui.add_space(6.0);

                let mut fps = self.config.max_fps as f32;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Max FPS").color(TEXT_DIM).font(FontId::proportional(13.0)));
                    ui.add_space(8.0);
                    ui.add(egui::Slider::new(&mut fps, 5.0..=60.0));
                });
                self.config.max_fps = fps as u32;

                ui.add_space(6.0);

                let displays = capture::list_displays();
                let mut monitor = self.config.capture_monitor;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Capture Monitor (mirror mode)").color(TEXT_DIM).font(FontId::proportional(13.0)));
                    ui.add_space(12.0);
                    egui::ComboBox::from_id_salt("monitor_select")
                        .selected_text(
                            displays
                                .get(monitor)
                                .cloned()
                                .unwrap_or_else(|| format!("Display {}", monitor)),
                        )
                        .show_ui(ui, |ui| {
                            for (i, d) in displays.iter().enumerate() {
                                ui.selectable_value(&mut monitor, i, d);
                            }
                        });
                });
                self.config.capture_monitor = monitor;
                if !self.config.use_virtual_display {
                    ui.label(
                        RichText::new(
                            "Tip: Display 0 is usually the primary/laptop screen. \
                             Change this if the wrong screen is being mirrored.",
                        )
                        .color(TEXT_DIM)
                        .font(FontId::proportional(11.0)),
                    );
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Virtual Display settings
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Virtual Display")
                            .color(TEXT)
                            .font(FontId::proportional(14.0)),
                    );
                    ui.add_space(8.0);
                    let toggle_text = if self.config.use_virtual_display {
                        "Extended (3rd screen)"
                    } else {
                        "Mirror mode"
                    };
                    if ui
                        .add(Button::new(
                            RichText::new(toggle_text)
                                .color(if self.config.use_virtual_display {
                                    GREEN
                                } else {
                                    TEXT_DIM
                                })
                                .font(FontId::proportional(12.0)),
                        ))
                        .clicked()
                    {
                        self.config.use_virtual_display = !self.config.use_virtual_display;
                    }
                });

                #[cfg(target_os = "windows")]
                if self.config.use_virtual_display {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "⚠ Extended mode on Windows requires the Virtual Display Driver.\n\
                             Install from: github.com/itsmikethetech/Virtual-Display-Driver/releases\n\
                             Without it, the app will fall back to mirror mode.",
                        )
                        .color(Color32::from_rgb(255, 180, 50))
                        .font(FontId::proportional(11.0)),
                    );
                }

                if self.config.use_virtual_display {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Creates a virtual monitor — the other laptop becomes an extra screen, not a mirror.",
                        )
                        .color(TEXT_DIM)
                        .font(FontId::proportional(11.0)),
                    );
                    ui.add_space(6.0);

                    let resolutions: Vec<(u32, u32, &str)> = vec![
                        (1280, 720, "1280x720 (HD)"),
                        (1920, 1080, "1920x1080 (Full HD)"),
                        (2560, 1440, "2560x1440 (QHD)"),
                        (3840, 2160, "3840x2160 (4K)"),
                    ];

                    let current_label = resolutions
                        .iter()
                        .find(|(w, h, _)| {
                            *w == self.config.virtual_display_width
                                && *h == self.config.virtual_display_height
                        })
                        .map(|(_, _, l)| l.to_string())
                        .unwrap_or_else(|| {
                            format!(
                                "{}x{}",
                                self.config.virtual_display_width,
                                self.config.virtual_display_height
                            )
                        });

                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Resolution")
                                .color(TEXT_DIM)
                                .font(FontId::proportional(13.0)),
                        );
                        ui.add_space(4.0);
                        egui::ComboBox::from_id_salt("vd_resolution")
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                for (w, h, label) in &resolutions {
                                    if ui
                                        .selectable_label(
                                            self.config.virtual_display_width == *w
                                                && self.config.virtual_display_height == *h,
                                            *label,
                                        )
                                        .clicked()
                                    {
                                        self.config.virtual_display_width = *w;
                                        self.config.virtual_display_height = *h;
                                    }
                                }
                            });
                    });
                }

                if self.virtual_monitor.is_some() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, GREEN);
                        ui.label(
                            RichText::new("Virtual display active")
                                .color(GREEN)
                                .font(FontId::proportional(12.0)),
                        );
                    });
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Auth key display + copy
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Auth Key").color(TEXT_DIM).font(FontId::proportional(13.0)));
                    ui.add_space(8.0);
                    let key_display = if self.config.auth_key.len() > 16 {
                        format!("{}...", &self.config.auth_key[..16])
                    } else {
                        self.config.auth_key.clone()
                    };
                    ui.label(
                        RichText::new(key_display)
                            .font(FontId::monospace(12.0))
                            .color(TEXT_BLUE),
                    );
                    let copy_label =
                        if self.auth_key_copied.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
                            "Copied!"
                        } else {
                            "Copy"
                        };
                    if ui
                        .add(Button::new(RichText::new(copy_label).color(TEXT_BLUE).font(FontId::proportional(12.0))))
                        .clicked()
                    {
                        ui.ctx().copy_text(self.config.auth_key.clone());
                        self.auth_key_copied = Some(Instant::now());
                    }
                });
                ui.add_space(4.0);
                // Paste key from the other machine
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Paste Key").color(TEXT_DIM).font(FontId::proportional(13.0)));
                    ui.add_space(4.0);
                    let mut key_edit = self.config.auth_key.clone();
                    let te = egui::TextEdit::singleline(&mut key_edit)
                        .font(FontId::monospace(11.0))
                        .desired_width(ui.available_width() - 8.0);
                    if ui.add(te).changed() {
                        self.config.auth_key = key_edit;
                    }
                });

                ui.add_space(12.0);
                if ui
                    .add(
                        Button::new(RichText::new("Save Settings").color(TEXT))
                            .fill(SURFACE_LIGHT)
                            .corner_radius(CornerRadius::same(6)),
                    )
                    .clicked()
                {
                    if let Err(e) = save_config(&self.config) {
                        self.push_log(format!("Save error: {}", e));
                    } else {
                        self.push_log("Settings saved");
                    }
                }
            });
        });
    }

    // ── Log Panel ───────────────────────────────────────────────────────

    fn draw_logs(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            card(ui, SURFACE, |ui| {
                ui.label(
                    RichText::new("Log")
                        .color(TEXT_DIM)
                        .font(FontId::proportional(13.0)),
                );
                ui.add_space(6.0);

                let start = if self.logs.len() > 8 {
                    self.logs.len() - 8
                } else {
                    0
                };
                for entry in &self.logs[start..] {
                    ui.label(
                        RichText::new(&entry.msg)
                            .font(FontId::monospace(11.0))
                            .color(TEXT_DIM),
                    );
                }
            });
        });
    }
}

// ── Reusable UI Components ──────────────────────────────────────────────────

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = SURFACE;
    visuals.faint_bg_color = SURFACE_LIGHT;
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.inactive.bg_fill = SURFACE_LIGHT;
    visuals.widgets.hovered.bg_fill = BORDER;
    visuals.widgets.active.bg_fill = ACCENT_BLUE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.selection.bg_fill = ACCENT_BLUE;
    visuals.selection.stroke = Stroke::new(1.0, TEXT);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    ctx.set_global_style(style);
}

fn card(ui: &mut egui::Ui, bg: Color32, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(12))
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::same(20))
        .outer_margin(Margin::symmetric(16, 0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

fn mode_card(
    ui: &mut egui::Ui,
    width: f32,
    title: &str,
    description: &str,
    color: Color32,
    hover_color: Color32,
    subtitle: &str,
) -> bool {
    let desired = Vec2::new(width, 160.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());

    let hovered = response.hovered();
    let bg = if hovered { hover_color } else { color };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(16), bg);

    painter.text(
        rect.center_top() + Vec2::new(0.0, 36.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(22.0),
        TEXT,
    );
    painter.text(
        rect.center_top() + Vec2::new(0.0, 68.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(12.0),
        Color32::from_white_alpha(180),
    );
    painter.text(
        rect.center_top() + Vec2::new(0.0, 110.0),
        Align2::CENTER_CENTER,
        description,
        FontId::proportional(12.0),
        Color32::from_white_alpha(140),
    );

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    response.clicked()
}

fn styled_button(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    hover: Color32,
    width: f32,
) -> Response {
    let btn = Button::new(RichText::new(label).color(TEXT).font(FontId::proportional(14.0)))
        .fill(color)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(width, 36.0));
    let resp = ui.add(btn);
    if resp.hovered() {
        ui.painter().rect_filled(
            resp.rect,
            CornerRadius::same(8),
            hover,
        );
        // Re-draw text on top of hover fill
        ui.painter().text(
            resp.rect.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(14.0),
            TEXT,
        );
    }
    resp
}

fn stat_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}:", label))
                .color(TEXT_DIM)
                .font(FontId::proportional(13.0)),
        );
        ui.label(
            RichText::new(value)
                .color(TEXT)
                .font(FontId::proportional(13.0)),
        );
    });
}

// ── Key Mapping ─────────────────────────────────────────────────────────────

fn egui_key_to_code(key: &egui::Key) -> Option<u32> {
    use egui::Key::*;
    Some(match key {
        Backspace => 3,
        Tab => 34,
        Enter => 29,
        Escape => 10,
        Space => 33,
        Delete => 7,
        Home => 23,
        End => 9,
        PageUp => 28,
        PageDown => 27,
        ArrowLeft => 24,
        ArrowRight => 30,
        ArrowUp => 35,
        ArrowDown => 8,
        A => 65,
        B => 81,
        C => 79,
        D => 67,
        E => 55,
        F => 68,
        G => 69,
        H => 70,
        I => 60,
        J => 71,
        K => 72,
        L => 73,
        M => 83,
        N => 82,
        O => 61,
        P => 62,
        Q => 53,
        R => 56,
        S => 66,
        T => 57,
        U => 59,
        V => 80,
        W => 54,
        X => 78,
        Y => 58,
        Z => 77,
        Num0 => 50,
        Num1 => 41,
        Num2 => 42,
        Num3 => 43,
        Num4 => 44,
        Num5 => 45,
        Num6 => 46,
        Num7 => 47,
        Num8 => 48,
        Num9 => 49,
        F1 => 11,
        F2 => 12,
        F3 => 13,
        F4 => 14,
        F5 => 15,
        F6 => 16,
        F7 => 17,
        F8 => 18,
        F9 => 19,
        F10 => 20,
        F11 => 21,
        F12 => 22,
        Minus => 51,
        _ => return None,
    })
}

// ── Background Network Threads ──────────────────────────────────────────────

fn primary_network_loop(
    frame_rx: Receiver<crate::capture::CapturedFrame>,
    port: u16,
    auth_hash: Vec<u8>,
    running: Arc<AtomicBool>,
    fps_counter: Arc<Mutex<u32>>,
    peer_name: Arc<Mutex<Option<String>>>,
    log_tx: Sender<String>,
) {
    // Use socket2 for SO_REUSEADDR to avoid "address already in use" on quick restart
    let listener = match (|| -> std::io::Result<std::net::TcpListener> {
        let sock = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        sock.set_reuse_address(true)?;
        #[cfg(target_os = "macos")]
        sock.set_reuse_port(true)?;
        sock.bind(&format!("0.0.0.0:{}", port).parse::<std::net::SocketAddr>().unwrap().into())?;
        sock.listen(4)?;
        sock.set_nonblocking(true)?;
        Ok(sock.into())
    })() {
        Ok(l) => l,
        Err(e) => {
            let _ = log_tx.try_send(format!("BIND FAILED on port {}: {} — is another instance running?", port, e));
            return;
        }
    };

    // Report the local IPs so the user knows what to connect to
    let local_ip = crate::discovery::get_local_ip();
    let _ = log_tx.try_send(format!("✓ Listening on {}:{} — waiting for display to connect", local_ip, port));

    let mut fps_count = 0u32;
    let mut fps_timer = Instant::now();

    while running.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let _ = log_tx.try_send(format!("Display connecting from {}", addr));
                stream.set_nonblocking(false).ok();
                let stream2 = match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let mut writer = std::io::BufWriter::new(stream);
                let mut reader = std::io::BufReader::new(stream2);

                // Handshake
                match protocol::read_message_sync(&mut reader) {
                    Ok(Message::Handshake {
                        auth_hash: remote_hash,
                        hostname,
                        ..
                    }) => {
                        if remote_hash != auth_hash {
                            let _ = log_tx.try_send(format!("Auth failed from {}", hostname));
                            let reject = Message::HandshakeAck { accepted: false };
                            let _ = protocol::write_message_sync(&mut writer, &reject);
                            continue;
                        }
                        let accept = Message::HandshakeAck { accepted: true };
                        let _ = protocol::write_message_sync(&mut writer, &accept);
                        *peer_name.lock().unwrap() = Some(hostname.clone());
                        let _ = log_tx.try_send(format!("Authenticated: {}", hostname));
                    }
                    _ => continue,
                }

                // Stream frames
                while running.load(Ordering::Relaxed) {
                    match frame_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(frame) => {
                            let msg = Message::Frame {
                                width: frame.width,
                                height: frame.height,
                                jpeg_data: frame.jpeg_data,
                            };
                            if protocol::write_message_sync(&mut writer, &msg).is_err() {
                                let _ = log_tx.try_send("Display disconnected".to_string());
                                break;
                            }
                            fps_count += 1;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }

                    if fps_timer.elapsed() >= Duration::from_secs(1) {
                        *fps_counter.lock().unwrap() = fps_count;
                        fps_count = 0;
                        fps_timer = Instant::now();
                    }
                }

                *peer_name.lock().unwrap() = None;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(200));
            }
            Err(e) => {
                let _ = log_tx.try_send(format!("Accept error: {}", e));
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

fn display_network_loop(
    peers: PeerMap,
    auth_hash: Vec<u8>,
    hostname: String,
    running: Arc<AtomicBool>,
    pixel_tx: Sender<DecodedFrame>,
    input_rx: Receiver<Message>,
    fps_counter: Arc<Mutex<u32>>,
    connected: Arc<AtomicBool>,
    log_tx: Sender<String>,
) {
    let mut fps_count = 0u32;
    let mut fps_timer = Instant::now();

    let mut search_logged = false;

    while running.load(Ordering::Relaxed) {
        let peer = match discovery::find_peer(&peers) {
            Some(p) => {
                search_logged = false;
                p
            }
            None => {
                if !search_logged {
                    let _ = log_tx.try_send("Searching for primary on network...".to_string());
                    search_logged = true;
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let target = format!("{}:{}", peer.ip, peer.stream_port);
        let _ = log_tx.try_send(format!(
            "Found {} — connecting to {}...",
            peer.hostname, target
        ));

        let stream = match TcpStream::connect_timeout(
            &target.parse().unwrap(),
            Duration::from_secs(5),
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = log_tx.try_send(format!(
                    "TCP connect to {} failed: {} — check firewall on primary machine",
                    target, e
                ));
                thread::sleep(Duration::from_secs(3));
                continue;
            }
        };

        stream.set_nonblocking(false).ok();
        let stream2 = match stream.try_clone() {
            Ok(s) => s,
            Err(_) => continue,
        };

        let mut writer = std::io::BufWriter::new(stream);
        let mut reader = std::io::BufReader::new(stream2);

        // Handshake
        let hs = Message::Handshake {
            version: PROTOCOL_VERSION,
            auth_hash: auth_hash.clone(),
            hostname: hostname.clone(),
        };
        if protocol::write_message_sync(&mut writer, &hs).is_err() {
            continue;
        }
        match protocol::read_message_sync(&mut reader) {
            Ok(Message::HandshakeAck { accepted: true }) => {
                let _ = log_tx.try_send("Authenticated with primary".to_string());
                connected.store(true, Ordering::Relaxed);
            }
            _ => {
                let _ = log_tx.try_send("Auth rejected".to_string());
                continue;
            }
        }

        // Spawn input sender thread
        let input_running = running.clone();
        let input_rx_clone = input_rx.clone();
        let input_sender = thread::spawn(move || {
            while input_running.load(Ordering::Relaxed) {
                match input_rx_clone.recv_timeout(Duration::from_millis(100)) {
                    Ok(msg) => {
                        if protocol::write_message_sync(&mut writer, &msg).is_err() {
                            break;
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
        });

        // Receive frames
        while running.load(Ordering::Relaxed) {
            match protocol::read_message_sync(&mut reader) {
                Ok(Message::Frame {
                    jpeg_data, ..
                }) => {
                    if let Some(decoded) = decode_frame(&jpeg_data) {
                        let _ = pixel_tx.try_send(decoded);
                        fps_count += 1;
                    }
                }
                Ok(Message::Heartbeat { .. }) => {}
                Ok(_) => {}
                Err(e) => {
                    let _ = log_tx.try_send(format!("Connection lost: {}", e));
                    break;
                }
            }

            if fps_timer.elapsed() >= Duration::from_secs(1) {
                *fps_counter.lock().unwrap() = fps_count;
                fps_count = 0;
                fps_timer = Instant::now();
            }
        }

        connected.store(false, Ordering::Relaxed);
        let _ = input_sender.join();
        let _ = log_tx.try_send("Disconnected from primary".to_string());
        thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(feature = "fast-jpeg")]
fn decode_frame(jpeg_data: &[u8]) -> Option<DecodedFrame> {
    let header = turbojpeg::read_header(jpeg_data).ok()?;
    let w = header.width;
    let h = header.height;

    let mut decompressor = turbojpeg::Decompressor::new().ok()?;
    let mut rgb = vec![0u8; w * h * 3];
    let image = turbojpeg::Image {
        pixels: rgb.as_mut_slice(),
        width: w,
        pitch: w * 3,
        height: h,
        format: turbojpeg::PixelFormat::RGB,
    };
    decompressor.decompress(jpeg_data, image).ok()?;

    let pixels: Vec<Color32> = rgb
        .chunks_exact(3)
        .map(|c| Color32::from_rgb(c[0], c[1], c[2]))
        .collect();

    Some(DecodedFrame {
        width: w,
        height: h,
        pixels,
    })
}

#[cfg(not(feature = "fast-jpeg"))]
fn decode_frame(jpeg_data: &[u8]) -> Option<DecodedFrame> {
    use image::codecs::jpeg::JpegDecoder;
    use image::ImageDecoder;
    use std::io::Cursor;

    let cursor = Cursor::new(jpeg_data);
    let decoder = JpegDecoder::new(cursor).ok()?;
    let (w, h) = decoder.dimensions();
    let w = w as usize;
    let h = h as usize;

    let mut rgb = vec![0u8; decoder.total_bytes() as usize];
    decoder.read_image(&mut rgb).ok()?;

    let pixels: Vec<Color32> = rgb
        .chunks_exact(3)
        .map(|c| Color32::from_rgb(c[0], c[1], c[2]))
        .collect();

    Some(DecodedFrame {
        width: w,
        height: h,
        pixels,
    })
}
