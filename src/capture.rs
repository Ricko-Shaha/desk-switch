use anyhow::Result;
use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use log::{debug, info, warn};
use scrap::{Capturer, Display};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: u16,
    pub height: u16,
    pub jpeg_data: Vec<u8>,
}

#[allow(dead_code)]
struct RawFrame {
    width: usize,
    height: usize,
    bgra_data: Vec<u8>,
}

pub struct CaptureSession {
    pub frame_rx: Receiver<CapturedFrame>,
    running: Arc<AtomicBool>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl CaptureSession {
    pub fn start(monitor_index: usize, quality: u8, max_fps: u32) -> Result<Self> {
        let running = Arc::new(AtomicBool::new(true));
        let (frame_tx, frame_rx): (Sender<CapturedFrame>, Receiver<CapturedFrame>) = bounded(2);

        let running_c = running.clone();
        let h = thread::spawn(move || {
            capture_and_encode_loop(monitor_index, quality, max_fps, frame_tx, running_c);
        });

        Ok(Self {
            frame_rx,
            running,
            handles: vec![h],
        })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for h in self.handles.drain(..) {
            let _ = h.join();
        }
        info!("Capture session stopped");
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn capture_and_encode_loop(
    monitor_index: usize,
    quality: u8,
    max_fps: u32,
    frame_tx: Sender<CapturedFrame>,
    running: Arc<AtomicBool>,
) {
    let displays = match Display::all() {
        Ok(d) => d,
        Err(e) => {
            warn!("Failed to enumerate displays: {}", e);
            return;
        }
    };
    if monitor_index >= displays.len() {
        warn!("Monitor index {} out of range (found {})", monitor_index, displays.len());
        return;
    }

    let display = displays.into_iter().nth(monitor_index).unwrap();
    let width = display.width();
    let height = display.height();
    info!("Capturing display {} ({}x{})", monitor_index, width, height);

    match Capturer::new(display) {
        Ok(capturer) => {
            info!("Using scrap (fast) capture");
            scrap_capture_encode(capturer, width, height, quality, max_fps, &frame_tx, &running);
        }
        Err(e) => {
            warn!("scrap capture failed: {}", e);
            #[cfg(target_os = "macos")]
            {
                info!("Falling back to screencapture (slower but no permission needed)");
                screencapture_loop(width, height, max_fps, &frame_tx, &running);
            }
            #[cfg(not(target_os = "macos"))]
            {
                warn!("No fallback capture available on this platform");
            }
        }
    }
}

fn scrap_capture_encode(
    mut capturer: Capturer,
    width: usize,
    height: usize,
    quality: u8,
    max_fps: u32,
    frame_tx: &Sender<CapturedFrame>,
    running: &Arc<AtomicBool>,
) {
    let frame_interval = Duration::from_micros(1_000_000 / max_fps as u64);

    #[cfg(feature = "fast-jpeg")]
    let mut compressor = {
        let mut c = match turbojpeg::Compressor::new() {
            Ok(c) => c,
            Err(e) => { warn!("turbojpeg compressor failed: {}", e); return; }
        };
        let _ = c.set_quality(quality as i32);
        let _ = c.set_subsamp(turbojpeg::Subsamp::Sub2x2);
        c
    };

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        match capturer.frame() {
            Ok(frame) => {
                let bgra_data = frame.to_vec();

                #[cfg(feature = "fast-jpeg")]
                let jpeg_result = {
                    let image = turbojpeg::Image {
                        pixels: bgra_data.as_slice(),
                        width,
                        pitch: width * 4,
                        height,
                        format: turbojpeg::PixelFormat::BGRA,
                    };
                    compressor.compress_to_vec(image).ok()
                };

                #[cfg(not(feature = "fast-jpeg"))]
                let jpeg_result = {
                    use image::codecs::jpeg::JpegEncoder;
                    use image::{ColorType, ImageEncoder};
                    use std::io::Cursor;

                    let mut rgb = Vec::with_capacity(width * height * 3);
                    for px in bgra_data.chunks_exact(4) {
                        rgb.push(px[2]);
                        rgb.push(px[1]);
                        rgb.push(px[0]);
                    }
                    let mut buf = Cursor::new(Vec::with_capacity(256 * 1024));
                    let enc = JpegEncoder::new_with_quality(&mut buf, quality);
                    enc.write_image(&rgb, width as u32, height as u32, ColorType::Rgb8.into())
                        .ok()
                        .map(|_| buf.into_inner())
                };

                if let Some(jpeg_data) = jpeg_result {
                    let cf = CapturedFrame {
                        width: width as u16,
                        height: height as u16,
                        jpeg_data,
                    };
                    match frame_tx.try_send(cf) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => { debug!("Network busy, dropping frame"); }
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(e) => {
                warn!("Capture error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }

        let elapsed = start.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }

    info!("scrap capture stopped");
}

#[cfg(target_os = "macos")]
fn screencapture_loop(
    width: usize,
    height: usize,
    max_fps: u32,
    frame_tx: &Sender<CapturedFrame>,
    running: &Arc<AtomicBool>,
) {
    let tmp_path = "/tmp/.deskswitch_capture.jpg";
    let effective_fps = max_fps.min(8);
    let frame_interval = Duration::from_millis(1000 / effective_fps as u64);

    info!("screencapture fallback started (target: ~{} FPS)", effective_fps);

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        let output = std::process::Command::new("screencapture")
            .args(["-x", "-t", "jpg", tmp_path])
            .output();

        match output {
            Ok(o) if o.status.success() => {
                if let Ok(jpeg_data) = std::fs::read(tmp_path) {
                    let frame = CapturedFrame {
                        width: width as u16,
                        height: height as u16,
                        jpeg_data,
                    };
                    match frame_tx.try_send(frame) {
                        Ok(()) => {}
                        Err(TrySendError::Full(_)) => { debug!("Network busy, dropping frame"); }
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
            }
            Ok(o) => {
                warn!("screencapture failed: {}", String::from_utf8_lossy(&o.stderr));
                thread::sleep(Duration::from_secs(1));
            }
            Err(e) => {
                warn!("screencapture error: {}", e);
                thread::sleep(Duration::from_secs(1));
            }
        }

        let elapsed = start.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }

    std::fs::remove_file(tmp_path).ok();
    info!("screencapture fallback stopped");
}

pub fn list_displays() -> Vec<String> {
    match Display::all() {
        Ok(displays) => displays
            .iter()
            .enumerate()
            .map(|(i, d)| format!("Display {}: {}x{}", i, d.width(), d.height()))
            .collect(),
        Err(e) => {
            warn!("Failed to enumerate displays: {}", e);
            vec![]
        }
    }
}
