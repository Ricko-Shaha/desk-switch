use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use image::codecs::jpeg::JpegDecoder;
use image::ImageDecoder;
use log::{debug, info, warn};
use minifb::{Key, Window, WindowOptions};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub struct DecodedFrame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

pub struct ViewerSession {
    pub jpeg_tx: Sender<(u16, u16, Vec<u8>)>,
    running: Arc<AtomicBool>,
    decode_handle: Option<thread::JoinHandle<()>>,
    render_handle: Option<thread::JoinHandle<()>>,
}

impl ViewerSession {
    pub fn start(running: Arc<AtomicBool>) -> Self {
        let (jpeg_tx, jpeg_rx): (Sender<(u16, u16, Vec<u8>)>, Receiver<(u16, u16, Vec<u8>)>) =
            bounded(2);
        let (pixel_tx, pixel_rx): (Sender<DecodedFrame>, Receiver<DecodedFrame>) = bounded(2);

        let running_decode = running.clone();
        let decode_handle = thread::spawn(move || {
            decode_loop(jpeg_rx, pixel_tx, running_decode);
        });

        let running_render = running.clone();
        let render_handle = thread::spawn(move || {
            render_loop(pixel_rx, running_render);
        });

        Self {
            jpeg_tx,
            running,
            decode_handle: Some(decode_handle),
            render_handle: Some(render_handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.decode_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.render_handle.take() {
            let _ = h.join();
        }
        info!("Viewer session stopped");
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for ViewerSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn decode_loop(
    jpeg_rx: Receiver<(u16, u16, Vec<u8>)>,
    pixel_tx: Sender<DecodedFrame>,
    running: Arc<AtomicBool>,
) {
    info!("JPEG decoder started");

    while running.load(Ordering::Relaxed) {
        match jpeg_rx.recv_timeout(Duration::from_millis(100)) {
            Ok((_width, _height, jpeg_data)) => {
                let start = Instant::now();
                match decode_jpeg(&jpeg_data) {
                    Ok(decoded) => {
                        debug!("Decoded frame in {:?}", start.elapsed());
                        match pixel_tx.try_send(decoded) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {
                                debug!("Renderer busy, dropping decoded frame");
                            }
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                    Err(e) => {
                        warn!("JPEG decode error: {}", e);
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }

    info!("JPEG decoder stopped");
}

fn decode_jpeg(jpeg_data: &[u8]) -> Result<DecodedFrame, Box<dyn std::error::Error>> {
    let cursor = Cursor::new(jpeg_data);
    let decoder = JpegDecoder::new(cursor)?;
    let (width, height) = decoder.dimensions();
    let width = width as usize;
    let height = height as usize;

    let mut rgb_data = vec![0u8; decoder.total_bytes() as usize];
    decoder.read_image(&mut rgb_data)?;

    // Convert RGB to u32 (0x00RRGGBB) for minifb
    let pixels: Vec<u32> = rgb_data
        .chunks_exact(3)
        .map(|rgb| ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | (rgb[2] as u32))
        .collect();

    Ok(DecodedFrame {
        width,
        height,
        pixels,
    })
}

fn render_loop(pixel_rx: Receiver<DecodedFrame>, running: Arc<AtomicBool>) {
    info!("Viewer render loop starting...");

    let mut window: Option<Window> = None;
    let mut current_width: usize = 0;
    let mut current_height: usize = 0;
    let mut last_frame: Option<DecodedFrame> = None;
    let mut fps_counter = 0u32;
    let mut fps_timer = Instant::now();
    while running.load(Ordering::Relaxed) {
        let mut new_frame = None;
        while let Ok(frame) = pixel_rx.try_recv() {
            new_frame = Some(frame);
        }

        if let Some(frame) = new_frame {
            if window.is_none() || frame.width != current_width || frame.height != current_height {
                current_width = frame.width;
                current_height = frame.height;

                let opts = WindowOptions {
                    borderless: true,
                    resize: true,
                    scale: minifb::Scale::FitScreen,
                    ..WindowOptions::default()
                };

                match Window::new(
                    "Desk Switch - Display Mode (ESC to exit)",
                    current_width,
                    current_height,
                    opts,
                ) {
                    Ok(mut w) => {
                        w.set_target_fps(60);
                        window = Some(w);
                        info!(
                            "Viewer window created: {}x{}",
                            current_width, current_height
                        );
                    }
                    Err(e) => {
                        warn!("Failed to create viewer window: {}", e);
                        continue;
                    }
                }
            }

            last_frame = Some(frame);
            fps_counter += 1;
        }

        if fps_timer.elapsed() >= Duration::from_secs(1) {
            debug!("Viewer FPS: {}", fps_counter);
            fps_counter = 0;
            fps_timer = Instant::now();
        }

        // Render
        if let Some(ref win) = window {
            if !win.is_open() || win.is_key_down(Key::Escape) {
                info!("Viewer window closed by user");
                running.store(false, Ordering::Relaxed);
                break;
            }
        }

        if let (Some(ref mut win), Some(ref frame)) = (&mut window, &last_frame) {
            if let Err(e) = win.update_with_buffer(&frame.pixels, frame.width, frame.height) {
                warn!("Viewer render error: {}", e);
            }
        } else {
            thread::sleep(Duration::from_millis(16));
        }
    }

    info!("Viewer render loop stopped");
}
