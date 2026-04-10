use anyhow::{anyhow, Result};
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

pub struct RawFrame {
    pub width: usize,
    pub height: usize,
    pub bgra_data: Vec<u8>,
}

pub struct CaptureSession {
    pub frame_rx: Receiver<CapturedFrame>,
    running: Arc<AtomicBool>,
    capture_handle: Option<thread::JoinHandle<()>>,
    encode_handle: Option<thread::JoinHandle<()>>,
}

impl CaptureSession {
    pub fn start(monitor_index: usize, quality: u8, max_fps: u32) -> Result<Self> {
        let running = Arc::new(AtomicBool::new(true));

        let (raw_tx, raw_rx): (Sender<RawFrame>, Receiver<RawFrame>) = bounded(2);
        let (frame_tx, frame_rx): (Sender<CapturedFrame>, Receiver<CapturedFrame>) = bounded(2);

        let running_capture = running.clone();
        let capture_handle = thread::spawn(move || {
            if let Err(e) = capture_loop(monitor_index, max_fps, raw_tx, running_capture) {
                warn!("Capture loop ended: {}", e);
            }
        });

        let running_encode = running.clone();
        let encode_handle = thread::spawn(move || {
            encode_loop(quality, raw_rx, frame_tx, running_encode);
        });

        Ok(Self {
            frame_rx,
            running,
            capture_handle: Some(capture_handle),
            encode_handle: Some(encode_handle),
        })
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.capture_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.encode_handle.take() {
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

fn capture_loop(
    monitor_index: usize,
    max_fps: u32,
    raw_tx: Sender<RawFrame>,
    running: Arc<AtomicBool>,
) -> Result<()> {
    let displays = Display::all().map_err(|e| anyhow!("Failed to enumerate displays: {}", e))?;
    if monitor_index >= displays.len() {
        return Err(anyhow!(
            "Monitor index {} out of range (found {} displays)",
            monitor_index,
            displays.len()
        ));
    }

    let display = displays.into_iter().nth(monitor_index).unwrap();
    let width = display.width();
    let height = display.height();
    info!("Capturing display {} ({}x{})", monitor_index, width, height);

    let mut capturer =
        Capturer::new(display).map_err(|e| anyhow!("Failed to create capturer: {}", e))?;

    let frame_interval = Duration::from_micros(1_000_000 / max_fps as u64);

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        match capturer.frame() {
            Ok(frame) => {
                let bgra_data = frame.to_vec();
                let raw = RawFrame {
                    width,
                    height,
                    bgra_data,
                };

                match raw_tx.try_send(raw) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        debug!("Encoder busy, dropping frame");
                    }
                    Err(TrySendError::Disconnected(_)) => break,
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

    Ok(())
}

#[cfg(feature = "fast-jpeg")]
fn encode_loop(
    quality: u8,
    raw_rx: Receiver<RawFrame>,
    frame_tx: Sender<CapturedFrame>,
    running: Arc<AtomicBool>,
) {
    info!("turbojpeg encoder started (quality: {})", quality);

    let mut compressor = match turbojpeg::Compressor::new() {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to create turbojpeg compressor: {}", e);
            return;
        }
    };
    let _ = compressor.set_quality(quality as i32);
    let _ = compressor.set_subsamp(turbojpeg::Subsamp::Sub2x2);

    while running.load(Ordering::Relaxed) {
        match raw_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(raw) => {
                let start = Instant::now();

                let image = turbojpeg::Image {
                    pixels: raw.bgra_data.as_slice(),
                    width: raw.width,
                    pitch: raw.width * 4,
                    height: raw.height,
                    format: turbojpeg::PixelFormat::BGRA,
                };

                match compressor.compress_to_vec(image) {
                    Ok(jpeg_data) => {
                        let encode_time = start.elapsed();
                        debug!(
                            "Encoded frame: {}x{} -> {} KB in {:?}",
                            raw.width,
                            raw.height,
                            jpeg_data.len() / 1024,
                            encode_time
                        );

                        let frame = CapturedFrame {
                            width: raw.width as u16,
                            height: raw.height as u16,
                            jpeg_data,
                        };

                        match frame_tx.try_send(frame) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                debug!("Network sender busy, dropping encoded frame");
                            }
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    Err(e) => {
                        warn!("JPEG encode error: {}", e);
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    info!("turbojpeg encoder stopped");
}

#[cfg(not(feature = "fast-jpeg"))]
fn encode_loop(
    quality: u8,
    raw_rx: Receiver<RawFrame>,
    frame_tx: Sender<CapturedFrame>,
    running: Arc<AtomicBool>,
) {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ColorType, ImageEncoder};
    use std::io::Cursor;

    info!("image crate JPEG encoder started (quality: {})", quality);

    while running.load(Ordering::Relaxed) {
        match raw_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(raw) => {
                let start = Instant::now();

                let pixel_count = raw.width * raw.height;
                let mut rgb_data = Vec::with_capacity(pixel_count * 3);
                for pixel in raw.bgra_data.chunks_exact(4) {
                    rgb_data.push(pixel[2]); // R
                    rgb_data.push(pixel[1]); // G
                    rgb_data.push(pixel[0]); // B
                }

                let mut jpeg_buf = Cursor::new(Vec::with_capacity(256 * 1024));
                let encoder = JpegEncoder::new_with_quality(&mut jpeg_buf, quality);

                match encoder.write_image(
                    &rgb_data,
                    raw.width as u32,
                    raw.height as u32,
                    ColorType::Rgb8.into(),
                ) {
                    Ok(()) => {
                        let jpeg_data = jpeg_buf.into_inner();
                        debug!(
                            "Encoded frame: {}x{} -> {} KB in {:?}",
                            raw.width, raw.height, jpeg_data.len() / 1024, start.elapsed()
                        );

                        let frame = CapturedFrame {
                            width: raw.width as u16,
                            height: raw.height as u16,
                            jpeg_data,
                        };

                        match frame_tx.try_send(frame) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                debug!("Network sender busy, dropping encoded frame");
                            }
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    Err(e) => warn!("JPEG encode error: {}", e),
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    info!("JPEG encoder stopped");
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
