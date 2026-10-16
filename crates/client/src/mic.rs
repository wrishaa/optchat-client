//! The native microphone (replaces app/ui/dictation.ts's AudioWorklet): the default input device through cpal,
//! downmixed to mono and downsampled to pcm16 16 kHz by the worklet's box filter (each output sample is the average of
//! the input samples since the last one), delivered in 100 ms chunks (1600 samples, 3200 bytes) with their rms level.
//!
//! cpal's `Stream` is !Send on macOS, so it lives on a dedicated thread that owns it; `on_chunk` is called on that thread
//! (never on the real-time audio thread). `stop` flushes the last partial chunk, then returns once the thread is gone.
use anyhow::{anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use parking_lot::Mutex;
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

pub struct MicChunk {
    /// pcm16 LE 16 kHz mono, ~100 ms = 3200 bytes (the last one may be shorter)
    pub pcm: Vec<u8>,
    /// rms of the input over the chunk, 0..1 (speech is ~0.02–0.2)
    pub level: f32,
}

const CHUNK: usize = 1600; // 100 ms at 16 kHz

/// the worklet's downsampler, sample for sample
struct Down {
    ratio: f64,
    pos: f64,
    acc: f64,
    cnt: u32,
    sq: f64,
    buf: Vec<i16>,
}
impl Down {
    fn new(rate: u32) -> Self {
        Down { ratio: rate as f64 / 16_000.0, pos: 0.0, acc: 0.0, cnt: 0, sq: 0.0, buf: Vec::with_capacity(CHUNK) }
    }
    fn push(&mut self, v: f32, out: &mut Vec<MicChunk>) {
        let v = v as f64;
        self.acc += v;
        self.cnt += 1;
        self.sq += v * v;
        self.pos += 1.0;
        if self.pos >= self.ratio {
            self.pos -= self.ratio;
            let s = (self.acc / self.cnt as f64).clamp(-1.0, 1.0);
            self.acc = 0.0;
            self.cnt = 0;
            self.buf.push(if s < 0.0 { s * 32768.0 } else { s * 32767.0 } as i16); // truncates, like Int16Array
            if self.buf.len() == CHUNK {
                out.extend(self.flush());
            }
        }
    }
    fn flush(&mut self) -> Option<MicChunk> {
        if self.buf.is_empty() {
            return None;
        }
        let level = (self.sq / (self.buf.len() as f64 * self.ratio).max(1.0)).sqrt() as f32;
        let pcm = self.buf.iter().flat_map(|s| s.to_le_bytes()).collect();
        self.buf.clear();
        self.sq = 0.0;
        Some(MicChunk { pcm, level })
    }
}

enum Msg {
    Chunk(MicChunk),
    Stop,
}

pub struct Mic {
    tx: mpsc::Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

/// macOS's microphone permission for this process (or the app that started it): None where unknown
#[cfg(target_os = "macos")]
fn authorization() -> Option<isize> {
    use std::ffi::{c_char, c_void, CStr};
    // AVCaptureDevice.authorizationStatus(for: .audio) through the Objective-C runtime, without a binding crate:
    // 0 not determined (starting asks), 1 restricted, 2 denied, 3 authorized
    unsafe {
        let open = |p: &CStr| libc::dlopen(p.as_ptr(), libc::RTLD_LAZY);
        let objc = open(c"/usr/lib/libobjc.A.dylib");
        let av = open(c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation");
        if objc.is_null() || av.is_null() {
            return None;
        }
        let get_class = libc::dlsym(objc, c"objc_getClass".as_ptr());
        let sel = libc::dlsym(objc, c"sel_registerName".as_ptr());
        let send = libc::dlsym(objc, c"objc_msgSend".as_ptr());
        let media = libc::dlsym(av, c"AVMediaTypeAudio".as_ptr()) as *const *const c_void;
        if get_class.is_null() || sel.is_null() || send.is_null() || media.is_null() || (*media).is_null() {
            return None;
        }
        let get_class: extern "C" fn(*const c_char) -> *const c_void = std::mem::transmute(get_class);
        let sel: extern "C" fn(*const c_char) -> *const c_void = std::mem::transmute(sel);
        let send: extern "C" fn(*const c_void, *const c_void, *const c_void) -> isize = std::mem::transmute(send);
        let class = get_class(c"AVCaptureDevice".as_ptr());
        if class.is_null() {
            return None;
        }
        Some(send(class, sel(c"authorizationStatusForMediaType:".as_ptr()), *media))
    }
}
#[cfg(not(target_os = "macos"))]
fn authorization() -> Option<isize> {
    None
}

/// whether there is a default input device
pub fn input_available() -> bool {
    cpal::default_host().default_input_device().is_some()
}

fn stream_of<T>(device: &cpal::Device, config: &cpal::StreamConfig, down: Arc<Mutex<Down>>, tx: mpsc::Sender<Msg>) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let mut out = Vec::new();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            {
                let mut d = down.lock();
                for frame in data.chunks(channels) {
                    // the browser's downmix to one channel: the average
                    let v = frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / frame.len() as f32;
                    d.push(v, &mut out);
                }
            }
            for c in out.drain(..) {
                let _ = tx.send(Msg::Chunk(c));
            }
        },
        |_| {}, // a device that goes away just stops sending; the dictation finishes on its own
        None,
    )
}

fn readable(e: impl std::fmt::Display) -> anyhow::Error {
    let s = e.to_string();
    let l = s.to_lowercase();
    if l.contains("permission") || l.contains("not permitted") || l.contains("denied") || l.contains("unauthorized") {
        anyhow!("microphone access is blocked")
    } else if l.contains("no longer available") || l.contains("not available") {
        anyhow!("no microphone found")
    } else {
        anyhow!("microphone failed: {s}")
    }
}

fn open(down_tx: mpsc::Sender<Msg>) -> anyhow::Result<(cpal::Stream, Arc<Mutex<Down>>)> {
    let device = cpal::default_host().default_input_device().ok_or_else(|| anyhow!("no microphone found"))?;
    let supported = device.default_input_config().map_err(readable)?;
    let config: cpal::StreamConfig = supported.config();
    let down = Arc::new(Mutex::new(Down::new(config.sample_rate.0)));
    let d = down.clone();
    use cpal::SampleFormat as F;
    let stream = match supported.sample_format() {
        F::F32 => stream_of::<f32>(&device, &config, d, down_tx),
        F::I16 => stream_of::<i16>(&device, &config, d, down_tx),
        F::U16 => stream_of::<u16>(&device, &config, d, down_tx),
        F::I32 => stream_of::<i32>(&device, &config, d, down_tx),
        F::U32 => stream_of::<u32>(&device, &config, d, down_tx),
        F::I8 => stream_of::<i8>(&device, &config, d, down_tx),
        F::U8 => stream_of::<u8>(&device, &config, d, down_tx),
        F::F64 => stream_of::<f64>(&device, &config, d, down_tx),
        other => bail!("microphone failed: unsupported sample format {other}"),
    }
    .map_err(readable)?;
    stream.play().map_err(readable)?;
    Ok((stream, down))
}

impl Mic {
    /// Starts recording from the default input device. Errors: "no microphone found", "microphone access is blocked"
    /// (macOS permission denied or restricted), "microphone failed: …".
    pub fn start(on_chunk: Arc<dyn Fn(MicChunk) + Send + Sync>) -> anyhow::Result<Mic> {
        optchat_platform::prepare_microphone().map_err(|message| anyhow!(message))?;
        if let Some(1 | 2) = authorization() {
            bail!("microphone access is blocked");
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<anyhow::Result<()>>(1);
        let stream_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("optchat-mic".into())
            .spawn(move || {
                let (stream, down) = match open(stream_tx) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                while let Ok(Msg::Chunk(c)) = rx.recv() {
                    on_chunk(c);
                }
                // stopped: no more callbacks once the stream is gone, then what is left, then the partial chunk
                drop(stream);
                while let Ok(m) = rx.try_recv() {
                    if let Msg::Chunk(c) = m {
                        on_chunk(c);
                    }
                }
                let last = down.lock().flush();
                if let Some(c) = last {
                    on_chunk(c);
                }
            })
            .map_err(|e| anyhow!("microphone failed: {e}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Mic { tx, thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(anyhow!("microphone failed"))
            }
        }
    }

    /// Flushes the last partial chunk (delivered before this returns), then stops the stream.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsample_like_the_worklet() {
        // 48 kHz -> 16 kHz: each output is the mean of 3 inputs; a 100 ms chunk is 4800 inputs
        let mut d = Down::new(48_000);
        let mut out = Vec::new();
        for i in 0..4800 + 300 {
            d.push(if i % 3 == 0 { 0.6 } else { 0.0 }, &mut out);
        }
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pcm.len(), 3200);
        let s0 = i16::from_le_bytes([out[0].pcm[0], out[0].pcm[1]]);
        assert_eq!(s0, (0.2f64 * 32767.0) as i16);
        assert!((out[0].level - (0.36f32 / 3.0).sqrt()).abs() < 1e-4);
        let last = d.flush().unwrap();
        assert_eq!(last.pcm.len(), 200);
        assert!(d.flush().is_none());
        // negative full scale, and clamping
        let mut d = Down::new(16_000);
        d.push(-1.5, &mut out);
        assert_eq!(d.buf, vec![-32768]);
    }

    #[test]
    fn enumerate_devices() {
        let host = cpal::default_host();
        let names: Vec<String> = host.input_devices().map(|ds| ds.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
        println!("inputs: {names:?}, default available: {}, authorization: {:?}", input_available(), authorization());
        assert_eq!(input_available(), host.default_input_device().is_some());
    }

    #[test]
    #[ignore = "records 1 s from the default microphone (needs permission)"]
    fn record_one_second() {
        let chunks = Arc::new(Mutex::new(Vec::<(usize, f32)>::new()));
        let c = chunks.clone();
        let mic = match Mic::start(Arc::new(move |m: MicChunk| c.lock().push((m.pcm.len(), m.level)))) {
            Ok(m) => m,
            Err(e) => panic!("{e}"),
        };
        std::thread::sleep(std::time::Duration::from_millis(1000));
        mic.stop();
        let got = chunks.lock().clone();
        println!("{} chunks: {got:?}", got.len());
        let bytes: usize = got.iter().map(|c| c.0).sum();
        assert!((9..=12).contains(&got.len()), "{} chunks", got.len());
        assert!((28_000..=36_000).contains(&bytes), "{bytes} bytes");
        assert!(got[..got.len() - 1].iter().all(|c| c.0 == 3200));
    }
}
