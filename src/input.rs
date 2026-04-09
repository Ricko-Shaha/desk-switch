use crate::protocol::Message;
use crossbeam_channel::{bounded, Receiver, Sender};
use enigo::{
    Axis, Button as EnigoButton, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings,
};
use log::{debug, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

pub struct InputCaptureSession {
    pub event_rx: Receiver<Message>,
    running: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl InputCaptureSession {
    pub fn start(running: Arc<AtomicBool>, screen_width: f64, screen_height: f64) -> Self {
        let (event_tx, event_rx) = bounded::<Message>(64);

        let running_clone = running.clone();
        let handle = thread::spawn(move || {
            capture_input_loop(event_tx, running_clone, screen_width, screen_height);
        });

        Self {
            event_rx,
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        info!("Input capture stopped");
    }
}

impl Drop for InputCaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn capture_input_loop(
    event_tx: Sender<Message>,
    running: Arc<AtomicBool>,
    screen_width: f64,
    screen_height: f64,
) {
    info!(
        "Input capture started (rdev listener, screen {}x{})",
        screen_width, screen_height
    );

    let tx = event_tx.clone();
    let running_cb = running.clone();

    let callback = move |event: rdev::Event| {
        if !running_cb.load(Ordering::Relaxed) {
            return;
        }

        let msg = match event.event_type {
            rdev::EventType::MouseMove { x, y } => {
                let xr = (x / screen_width).clamp(0.0, 1.0) as f32;
                let yr = (y / screen_height).clamp(0.0, 1.0) as f32;
                Some(Message::MouseMove {
                    x_ratio: xr,
                    y_ratio: yr,
                })
            }
            rdev::EventType::ButtonPress(button) => {
                let btn = rdev_button_to_u8(button);
                Some(Message::MouseClick {
                    button: btn,
                    pressed: true,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                })
            }
            rdev::EventType::ButtonRelease(button) => {
                let btn = rdev_button_to_u8(button);
                Some(Message::MouseClick {
                    button: btn,
                    pressed: false,
                    x_ratio: 0.0,
                    y_ratio: 0.0,
                })
            }
            rdev::EventType::Wheel { delta_x, delta_y } => Some(Message::MouseScroll {
                dx: delta_x as i32,
                dy: delta_y as i32,
            }),
            rdev::EventType::KeyPress(key) => {
                let code = rdev_key_to_u32(key);
                Some(Message::KeyEvent {
                    key_code: code,
                    pressed: true,
                })
            }
            rdev::EventType::KeyRelease(key) => {
                let code = rdev_key_to_u32(key);
                Some(Message::KeyEvent {
                    key_code: code,
                    pressed: false,
                })
            }
        };

        if let Some(msg) = msg {
            let _ = tx.try_send(msg);
        }
    };

    if let Err(e) = rdev::listen(callback) {
        warn!("rdev listen error: {:?}", e);
    }

    info!("Input capture loop ended");
}

pub struct InputSimulator {
    enigo: Enigo,
    screen_width: f32,
    screen_height: f32,
}

impl InputSimulator {
    pub fn new(screen_width: u32, screen_height: u32) -> Self {
        let enigo = Enigo::new(&Settings::default()).expect("Failed to create Enigo instance");
        Self {
            enigo,
            screen_width: screen_width as f32,
            screen_height: screen_height as f32,
        }
    }

    pub fn handle_message(&mut self, msg: &Message) {
        match msg {
            Message::MouseMove { x_ratio, y_ratio } => {
                let x = (*x_ratio * self.screen_width) as i32;
                let y = (*y_ratio * self.screen_height) as i32;
                if let Err(e) = self.enigo.move_mouse(x, y, Coordinate::Abs) {
                    debug!("Mouse move error: {:?}", e);
                }
            }
            Message::MouseClick {
                button,
                pressed,
                x_ratio: _,
                y_ratio: _,
            } => {
                let btn = u8_to_enigo_button(*button);
                let dir = if *pressed {
                    Direction::Press
                } else {
                    Direction::Release
                };
                if let Err(e) = self.enigo.button(btn, dir) {
                    debug!("Mouse click error: {:?}", e);
                }
            }
            Message::MouseScroll { dx, dy } => {
                if *dy != 0 {
                    if let Err(e) = self.enigo.scroll(*dy, Axis::Vertical) {
                        debug!("Scroll error: {:?}", e);
                    }
                }
                if *dx != 0 {
                    if let Err(e) = self.enigo.scroll(*dx, Axis::Horizontal) {
                        debug!("Scroll error: {:?}", e);
                    }
                }
            }
            Message::KeyEvent { key_code, pressed } => {
                if let Some(key) = u32_to_enigo_key(*key_code) {
                    let dir = if *pressed {
                        Direction::Press
                    } else {
                        Direction::Release
                    };
                    if let Err(e) = self.enigo.key(key, dir) {
                        debug!("Key event error: {:?}", e);
                    }
                }
            }
            _ => {}
        }
    }
}

fn rdev_button_to_u8(button: rdev::Button) -> u8 {
    match button {
        rdev::Button::Left => 0,
        rdev::Button::Right => 1,
        rdev::Button::Middle => 2,
        rdev::Button::Unknown(n) => n as u8,
    }
}

fn u8_to_enigo_button(b: u8) -> EnigoButton {
    match b {
        0 => EnigoButton::Left,
        1 => EnigoButton::Right,
        2 => EnigoButton::Middle,
        _ => EnigoButton::Left,
    }
}

fn rdev_key_to_u32(key: rdev::Key) -> u32 {
    match key {
        rdev::Key::Alt => 1,
        rdev::Key::AltGr => 2,
        rdev::Key::Backspace => 3,
        rdev::Key::CapsLock => 4,
        rdev::Key::ControlLeft => 5,
        rdev::Key::ControlRight => 6,
        rdev::Key::Delete => 7,
        rdev::Key::DownArrow => 8,
        rdev::Key::End => 9,
        rdev::Key::Escape => 10,
        rdev::Key::F1 => 11,
        rdev::Key::F2 => 12,
        rdev::Key::F3 => 13,
        rdev::Key::F4 => 14,
        rdev::Key::F5 => 15,
        rdev::Key::F6 => 16,
        rdev::Key::F7 => 17,
        rdev::Key::F8 => 18,
        rdev::Key::F9 => 19,
        rdev::Key::F10 => 20,
        rdev::Key::F11 => 21,
        rdev::Key::F12 => 22,
        rdev::Key::Home => 23,
        rdev::Key::LeftArrow => 24,
        rdev::Key::MetaLeft => 25,
        rdev::Key::MetaRight => 26,
        rdev::Key::PageDown => 27,
        rdev::Key::PageUp => 28,
        rdev::Key::Return => 29,
        rdev::Key::RightArrow => 30,
        rdev::Key::ShiftLeft => 31,
        rdev::Key::ShiftRight => 32,
        rdev::Key::Space => 33,
        rdev::Key::Tab => 34,
        rdev::Key::UpArrow => 35,
        rdev::Key::PrintScreen => 36,
        rdev::Key::ScrollLock => 37,
        rdev::Key::Pause => 38,
        rdev::Key::NumLock => 39,
        rdev::Key::BackQuote => 40,
        rdev::Key::Num1 => 41,
        rdev::Key::Num2 => 42,
        rdev::Key::Num3 => 43,
        rdev::Key::Num4 => 44,
        rdev::Key::Num5 => 45,
        rdev::Key::Num6 => 46,
        rdev::Key::Num7 => 47,
        rdev::Key::Num8 => 48,
        rdev::Key::Num9 => 49,
        rdev::Key::Num0 => 50,
        rdev::Key::Minus => 51,
        rdev::Key::Equal => 52,
        rdev::Key::KeyQ => 53,
        rdev::Key::KeyW => 54,
        rdev::Key::KeyE => 55,
        rdev::Key::KeyR => 56,
        rdev::Key::KeyT => 57,
        rdev::Key::KeyY => 58,
        rdev::Key::KeyU => 59,
        rdev::Key::KeyI => 60,
        rdev::Key::KeyO => 61,
        rdev::Key::KeyP => 62,
        rdev::Key::LeftBracket => 63,
        rdev::Key::RightBracket => 64,
        rdev::Key::KeyA => 65,
        rdev::Key::KeyS => 66,
        rdev::Key::KeyD => 67,
        rdev::Key::KeyF => 68,
        rdev::Key::KeyG => 69,
        rdev::Key::KeyH => 70,
        rdev::Key::KeyJ => 71,
        rdev::Key::KeyK => 72,
        rdev::Key::KeyL => 73,
        rdev::Key::SemiColon => 74,
        rdev::Key::Quote => 75,
        rdev::Key::BackSlash => 76,
        rdev::Key::KeyZ => 77,
        rdev::Key::KeyX => 78,
        rdev::Key::KeyC => 79,
        rdev::Key::KeyV => 80,
        rdev::Key::KeyB => 81,
        rdev::Key::KeyN => 82,
        rdev::Key::KeyM => 83,
        rdev::Key::Comma => 84,
        rdev::Key::Dot => 85,
        rdev::Key::Slash => 86,
        rdev::Key::Insert => 87,
        rdev::Key::IntlBackslash => 88,
        rdev::Key::Unknown(code) => 1000 + code as u32,
        _ => 999,
    }
}

fn u32_to_enigo_key(code: u32) -> Option<enigo::Key> {
    match code {
        1 => Some(enigo::Key::Alt),
        3 => Some(enigo::Key::Backspace),
        4 => Some(enigo::Key::CapsLock),
        5 => Some(enigo::Key::Control),
        6 => Some(enigo::Key::Control),
        7 => Some(enigo::Key::Delete),
        8 => Some(enigo::Key::DownArrow),
        9 => Some(enigo::Key::End),
        10 => Some(enigo::Key::Escape),
        11 => Some(enigo::Key::F1),
        12 => Some(enigo::Key::F2),
        13 => Some(enigo::Key::F3),
        14 => Some(enigo::Key::F4),
        15 => Some(enigo::Key::F5),
        16 => Some(enigo::Key::F6),
        17 => Some(enigo::Key::F7),
        18 => Some(enigo::Key::F8),
        19 => Some(enigo::Key::F9),
        20 => Some(enigo::Key::F10),
        21 => Some(enigo::Key::F11),
        22 => Some(enigo::Key::F12),
        23 => Some(enigo::Key::Home),
        24 => Some(enigo::Key::LeftArrow),
        25 => Some(enigo::Key::Meta),
        26 => Some(enigo::Key::Meta),
        27 => Some(enigo::Key::PageDown),
        28 => Some(enigo::Key::PageUp),
        29 => Some(enigo::Key::Return),
        30 => Some(enigo::Key::RightArrow),
        31 => Some(enigo::Key::Shift),
        32 => Some(enigo::Key::Shift),
        33 => Some(enigo::Key::Space),
        34 => Some(enigo::Key::Tab),
        35 => Some(enigo::Key::UpArrow),
        40 => Some(enigo::Key::Unicode('`')),
        41 => Some(enigo::Key::Unicode('1')),
        42 => Some(enigo::Key::Unicode('2')),
        43 => Some(enigo::Key::Unicode('3')),
        44 => Some(enigo::Key::Unicode('4')),
        45 => Some(enigo::Key::Unicode('5')),
        46 => Some(enigo::Key::Unicode('6')),
        47 => Some(enigo::Key::Unicode('7')),
        48 => Some(enigo::Key::Unicode('8')),
        49 => Some(enigo::Key::Unicode('9')),
        50 => Some(enigo::Key::Unicode('0')),
        51 => Some(enigo::Key::Unicode('-')),
        52 => Some(enigo::Key::Unicode('=')),
        53 => Some(enigo::Key::Unicode('q')),
        54 => Some(enigo::Key::Unicode('w')),
        55 => Some(enigo::Key::Unicode('e')),
        56 => Some(enigo::Key::Unicode('r')),
        57 => Some(enigo::Key::Unicode('t')),
        58 => Some(enigo::Key::Unicode('y')),
        59 => Some(enigo::Key::Unicode('u')),
        60 => Some(enigo::Key::Unicode('i')),
        61 => Some(enigo::Key::Unicode('o')),
        62 => Some(enigo::Key::Unicode('p')),
        63 => Some(enigo::Key::Unicode('[')),
        64 => Some(enigo::Key::Unicode(']')),
        65 => Some(enigo::Key::Unicode('a')),
        66 => Some(enigo::Key::Unicode('s')),
        67 => Some(enigo::Key::Unicode('d')),
        68 => Some(enigo::Key::Unicode('f')),
        69 => Some(enigo::Key::Unicode('g')),
        70 => Some(enigo::Key::Unicode('h')),
        71 => Some(enigo::Key::Unicode('j')),
        72 => Some(enigo::Key::Unicode('k')),
        73 => Some(enigo::Key::Unicode('l')),
        74 => Some(enigo::Key::Unicode(';')),
        75 => Some(enigo::Key::Unicode('\'')),
        76 => Some(enigo::Key::Unicode('\\')),
        77 => Some(enigo::Key::Unicode('z')),
        78 => Some(enigo::Key::Unicode('x')),
        79 => Some(enigo::Key::Unicode('c')),
        80 => Some(enigo::Key::Unicode('v')),
        81 => Some(enigo::Key::Unicode('b')),
        82 => Some(enigo::Key::Unicode('n')),
        83 => Some(enigo::Key::Unicode('m')),
        84 => Some(enigo::Key::Unicode(',')),
        85 => Some(enigo::Key::Unicode('.')),
        86 => Some(enigo::Key::Unicode('/')),
        _ => None,
    }
}
