#![cfg_attr(windows, windows_subsystem = "windows")]
use iced::keyboard::{self, Key};
use iced::widget::{button, center, column, row, scrollable, text};
use iced::{Element, Length, Subscription, Task, Theme, time};

use std::fs;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, SampleFormat, Stream, StreamConfig};
use hound::{WavReader, WavSpec};

fn write_wav_file_f32(path: &str, spec: WavSpec, samples: &[f32]) -> io::Result<()> {
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }
    writer
        .finalize()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(())
}

fn write_wav_file_i16(path: &str, spec: WavSpec, samples: &[i16]) -> io::Result<()> {
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    for &s in samples {
        writer
            .write_sample(s)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }
    writer
        .finalize()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(())
}

fn list_wav_files() -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.to_lowercase().ends_with(".wav") {
                    files.push(name.to_string());
                }
            }
        }
    }
    files.sort();
    files
}

#[derive(Debug, Clone)]
enum Message {
    StartRecording,
    StopRecording,
    PlayFile(String),
    StopPlayback,
    DeleteFile(String),
    Tick(Instant),
    Toggle,
    Reset,
    FinalizeRecording,
}

struct VoiceRecorder {
    is_recording: bool,
    is_playing: bool,
    status_message: String,
    files: Vec<String>,
    audio_data: Arc<Mutex<Vec<f32>>>,
    input_stream: Option<Stream>,
    output_stream: Option<Stream>,
    playback_status_tx: mpsc::Sender<()>,
    playback_status_rx: mpsc::Receiver<()>,
    start_time: Option<Instant>,
    elapsed_time: Duration,
    stopping_time: Option<Instant>,
    recording_sample_rate: u32, // NEW: Track actual recording sample rate
    recording_channels: u16,    // NEW: Track actual recording channels
}

impl Default for VoiceRecorder {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            is_recording: false,
            is_playing: false,
            status_message: "Ready to record.".into(),
            files: list_wav_files(),
            audio_data: Arc::new(Mutex::new(Vec::new())),
            input_stream: None,
            output_stream: None,
            playback_status_tx: tx,
            playback_status_rx: rx,
            start_time: None,
            elapsed_time: Duration::from_secs(0),
            stopping_time: None,
            recording_sample_rate: 48000, // NEW: Default sample rate
            recording_channels: 1,        // NEW: Default channels
        }
    }
}

impl VoiceRecorder {
    fn start_recording_impl(&mut self) {
        if self.is_recording {
            return;
        }

        self.audio_data.lock().unwrap().clear();
        let host = cpal::default_host();

        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                self.status_message = "No input device found.".into();
                return;
            }
        };

        let default_config = match device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                self.status_message = format!("Failed to get default input config: {}", e);
                return;
            }
        };

        let preferred_sample_rate = cpal::SampleRate(48000);
        let config = if default_config.sample_rate() <= preferred_sample_rate
            && preferred_sample_rate <= default_config.sample_rate()
        {
            StreamConfig {
                channels: default_config.channels(),
                sample_rate: preferred_sample_rate,
                buffer_size: BufferSize::Fixed(1024),
            }
        } else {
            StreamConfig {
                channels: default_config.channels(),
                sample_rate: default_config.sample_rate(),
                buffer_size: BufferSize::Fixed(1024),
            }
        };

        // NEW: Store the actual recording configuration
        self.recording_sample_rate = config.sample_rate.0;
        self.recording_channels = config.channels as u16;

        println!(
            "Recording with: channels={}, sample_rate={}, format={:?}",
            config.channels,
            config.sample_rate.0,
            default_config.sample_format()
        );

        let audio_buf = Arc::clone(&self.audio_data);

        let build_result = match default_config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    let mut buf = audio_buf.lock().unwrap();
                    buf.extend_from_slice(data);
                },
                move |err| {
                    eprintln!("Input stream error: {}", err);
                },
                None,
            ),
            SampleFormat::I16 => {
                let audio_buf = Arc::clone(&self.audio_data);
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _| {
                        let mut buf = audio_buf.lock().unwrap();
                        buf.extend(data.iter().map(|&s| (s as f32) / (i16::MAX as f32)));
                    },
                    move |err| {
                        eprintln!("Input stream error: {}", err);
                    },
                    None,
                )
            }
            SampleFormat::U16 => {
                let audio_buf = Arc::clone(&self.audio_data);
                device.build_input_stream(
                    &config,
                    move |data: &[u16], _| {
                        let mut buf = audio_buf.lock().unwrap();
                        buf.extend(
                            data.iter()
                                .map(|&s| (s as f32) / (u16::MAX as f32) * 2.0 - 1.0),
                        );
                    },
                    move |err| {
                        eprintln!("Input stream error: {}", err);
                    },
                    None,
                )
            }
            _ => {
                self.status_message = "Unsupported input sample format".into();
                return;
            }
        };

        match build_result {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    self.status_message = format!("Failed to start input stream: {}", e);
                    return;
                }
                self.input_stream = Some(stream);
                self.is_recording = true;
                self.status_message = "Recording...".into();
                self.start_time = Some(Instant::now());
                self.elapsed_time = Duration::from_secs(0);
                self.stopping_time = None;
            }
            Err(e) => {
                self.status_message = format!("Failed to build input stream: {}", e);
            }
        }
    }

    fn stop_recording_impl(&mut self) {
        if !self.is_recording {
            return;
        }

        self.is_recording = false;
        self.start_time = None;
        self.stopping_time = Some(Instant::now());
        self.status_message = "Stopping recording...".into();
    }

    fn finalize_recording(&mut self) {
        self.input_stream = None;
        self.stopping_time = None;

        let filename = format!("recording_{}.wav", self.files.len() + 1);
        let samples: Vec<f32> = std::mem::take(&mut *self.audio_data.lock().unwrap());

        if samples.is_empty() {
            self.status_message = "Error saving file: No audio data captured".into();
            return;
        }

        let spec = WavSpec {
            channels: self.recording_channels,
            sample_rate: self.recording_sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        println!(
            "Saving WAV file with: channels={}, sample_rate={}, bits_per_sample={}, format=F32",
            spec.channels, spec.sample_rate, spec.bits_per_sample
        );

        match write_wav_file_f32(&filename, spec, &samples) {
            Ok(()) => {
                self.status_message = format!("Recording saved as '{}'", filename);
                self.files = list_wav_files();
            }
            Err(e) => {
                self.status_message = format!("Error saving file: {}", e);
            }
        }
    }

    fn play_file_impl(&mut self, filename: &str) {
        if self.is_playing {
            return;
        }

        self.stop_playback_impl();

        let reader = match WavReader::open(filename) {
            Ok(r) => r,
            Err(e) => {
                self.status_message = format!("Error opening file: {}", e);
                return;
            }
        };

        let spec = reader.spec();
        println!(
            "File spec: channels={}, sample_rate={}, bits_per_sample={}",
            spec.channels, spec.sample_rate, spec.bits_per_sample
        );
        let samples = match spec.sample_format {
            hound::SampleFormat::Float => {
                match reader
                    .into_samples::<f32>()
                    .collect::<Result<Vec<f32>, _>>()
                {
                    Ok(s) => s,
                    Err(e) => {
                        self.status_message = format!("Error reading float samples: {}", e);
                        return;
                    }
                }
            }
            hound::SampleFormat::Int => match spec.bits_per_sample {
                16 => {
                    match reader
                        .into_samples::<i16>()
                        .collect::<Result<Vec<i16>, _>>()
                    {
                        Ok(samples_i16) => samples_i16
                            .into_iter()
                            .map(|s| s as f32 / i16::MAX as f32)
                            .collect(),
                        Err(e) => {
                            self.status_message = format!("Error reading i16 samples: {}", e);
                            return;
                        }
                    }
                }
                32 => {
                    match reader
                        .into_samples::<i32>()
                        .collect::<Result<Vec<i32>, _>>()
                    {
                        Ok(samples_i32) => samples_i32
                            .into_iter()
                            .map(|s| s as f32 / i32::MAX as f32)
                            .collect(),
                        Err(e) => {
                            self.status_message = format!("Error reading i32 samples: {}", e);
                            return;
                        }
                    }
                }
                _ => {
                    self.status_message =
                        format!("Unsupported bit depth: {}", spec.bits_per_sample);
                    return;
                }
            },
        };

        if samples.is_empty() {
            self.status_message = "File contains no samples.".into();
            return;
        }

        let samples_arc = Arc::new(Mutex::new(samples));
        let play_tx = self.playback_status_tx.clone();

        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                self.status_message = "Failed to find default output device".into();
                return;
            }
        };

        let supported_cfgs = match device.supported_output_configs() {
            Ok(v) => {
                let configs: Vec<_> = v.collect();
                println!("Supported output configs:");
                for cfg in &configs {
                    println!(
                        "  Channels: {}, Sample rate range: {}-{}, Format: {:?}",
                        cfg.channels(),
                        cfg.min_sample_rate().0,
                        cfg.max_sample_rate().0,
                        cfg.sample_format()
                    );
                }
                configs
            }
            Err(e) => {
                self.status_message = format!("Error querying output configs: {}", e);
                return;
            }
        };

        let matched = supported_cfgs
            .into_iter()
            .filter(|c| c.channels() == spec.channels as u16)
            .min_by_key(|c| {
                let format_priority = match c.sample_format() {
                    SampleFormat::F32 => 0,
                    SampleFormat::I16 => 1,
                    SampleFormat::I32 => 2,
                    SampleFormat::U16 => 3,
                    SampleFormat::U8 => 100,
                    _ => 50,
                };
                let rate_diff = ((c.max_sample_rate().0 as i64) - (spec.sample_rate as i64)).abs();
                (format_priority, rate_diff)
            });

        let chosen = match matched {
            Some(c) => {
                let sample_rate = if spec.sample_rate >= c.min_sample_rate().0
                    && spec.sample_rate <= c.max_sample_rate().0
                {
                    cpal::SampleRate(spec.sample_rate)
                } else {
                    c.max_sample_rate()
                };
                c.with_sample_rate(sample_rate)
            }
            None => {
                let fallback = device
                    .supported_output_configs()
                    .ok()
                    .and_then(|mut configs| configs.next());

                match fallback {
                    Some(c) => {
                        self.status_message = format!(
                            "Using fallback config (channels: {} -> {})",
                            spec.channels,
                            c.channels()
                        );
                        c.with_sample_rate(c.max_sample_rate())
                    }
                    None => {
                        self.status_message = "No compatible output configuration found.".into();
                        return;
                    }
                }
            }
        };

        let sample_format = chosen.sample_format();
        let stream_config: StreamConfig = chosen.into();

        println!(
            "File sample rate: {}, Device will use: {}",
            spec.sample_rate, stream_config.sample_rate.0
        );

        if spec.sample_rate != stream_config.sample_rate.0 {
            println!("WARNING: Sample rate mismatch detected! This may cause pitch issues.");
            self.status_message = format!(
                "Sample rate mismatch: file={}Hz, device={}Hz",
                spec.sample_rate, stream_config.sample_rate.0
            );
        }

        println!("Using sample format: {:?}", sample_format);

        let samples_for_callback = Arc::clone(&samples_arc);
        let play_tx_clone = play_tx.clone();

        let build_out = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |out: &mut [f32], _| {
                    let mut buf = samples_for_callback.lock().unwrap();
                    let len = out.len().min(buf.len());
                    if len > 0 {
                        out[..len].copy_from_slice(&buf[..len]);
                        buf.drain(..len);

                        if len < out.len() {
                            out[len..].fill(0.0);
                        }
                    } else {
                        out.fill(0.0);
                    }
                    if buf.is_empty() {
                        let _ = play_tx_clone.send(());
                    }
                },
                move |err| eprintln!("Output stream error: {}", err),
                None,
            ),
            SampleFormat::I16 => {
                let samples_for_callback = Arc::clone(&samples_arc);
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [i16], _| {
                        let mut buf = samples_for_callback.lock().unwrap();
                        let len = out.len().min(buf.len());
                        for i in 0..len {
                            out[i] = (buf[i].clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                        if len < out.len() {
                            out[len..].fill(0);
                        }
                        if buf.len() >= len {
                            buf.drain(..len);
                        } else {
                            buf.clear();
                        }
                        if buf.is_empty() {
                            let _ = play_tx_clone.send(());
                        }
                    },
                    move |err| eprintln!("Output stream error: {}", err),
                    None,
                )
            }
            SampleFormat::U16 => {
                let samples_for_callback = Arc::clone(&samples_arc);
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [u16], _| {
                        let mut buf = samples_for_callback.lock().unwrap();
                        let len = out.len().min(buf.len());
                        for i in 0..len {
                            let v = ((buf[i].clamp(-1.0, 1.0) + 1.0) * 0.5 * u16::MAX as f32);
                            out[i] = v as u16;
                        }
                        if len < out.len() {
                            out[len..].fill(u16::MAX / 2);
                        }
                        if buf.len() >= len {
                            buf.drain(..len);
                        } else {
                            buf.clear();
                        }
                        if buf.is_empty() {
                            let _ = play_tx_clone.send(());
                        }
                    },
                    move |err| eprintln!("Output stream error: {}", err),
                    None,
                )
            }
            SampleFormat::U8 => {
                let samples_for_callback = Arc::clone(&samples_arc);
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [u8], _| {
                        let mut buf = samples_for_callback.lock().unwrap();
                        let len = out.len().min(buf.len());
                        for i in 0..len {
                            let sample = buf[i].clamp(-1.0, 1.0);
                            let scaled = (sample + 1.0) * 127.5;

                            let dithered = scaled + ((i as f32 * 0.618033988749) % 1.0 - 0.5);
                            out[i] = dithered.clamp(0.0, 255.0) as u8;
                        }
                        if len < out.len() {
                            out[len..].fill(128);
                        }
                        if buf.len() >= len {
                            buf.drain(..len);
                        } else {
                            buf.clear();
                        }
                        if buf.is_empty() {
                            let _ = play_tx_clone.send(());
                        }
                    },
                    move |err| eprintln!("Output stream error: {}", err),
                    None,
                )
            }
            _ => {
                self.status_message =
                    format!("Unsupported output sample format: {:?}", sample_format);
                return;
            }
        };

        match build_out {
            Ok(stream) => {
                if let Err(e) = stream.play() {
                    self.status_message = format!("Failed to start output stream: {}", e);
                    return;
                }
                self.output_stream = Some(stream);
                self.is_playing = true;
                self.status_message = format!("Playing: {}", filename);
                self.start_time = Some(Instant::now());
                self.elapsed_time = Duration::from_secs(0);
            }
            Err(e) => {
                self.status_message = format!("Failed to build output stream: {}", e);
            }
        }
    }

    fn stop_playback_impl(&mut self) {
        if self.is_playing {
            self.output_stream = None;
            self.is_playing = false;
            self.status_message = "Playback stopped.".into();
            self.start_time = None;
            self.elapsed_time = Duration::from_secs(0);
        }
    }

    fn delete_file_impl(&mut self, filename: &str) {
        match fs::remove_file(filename) {
            Ok(_) => {
                self.status_message = format!("Deleted file: {}", filename);
                self.files = list_wav_files();
            }
            Err(e) => {
                self.status_message = format!("Error deleting file: {}", e);
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StartRecording => self.start_recording_impl(),
            Message::StopRecording => self.stop_recording_impl(),
            Message::PlayFile(fname) => self.play_file_impl(&fname),
            Message::StopPlayback => self.stop_playback_impl(),
            Message::DeleteFile(fname) => self.delete_file_impl(&fname),
            Message::FinalizeRecording => self.finalize_recording(),
            Message::Tick(now) => {
                if let Some(start) = self.start_time {
                    self.elapsed_time = now - start;
                }

                if let Some(stop_time) = self.stopping_time {
                    if now.duration_since(stop_time) >= Duration::from_millis(200) {
                        return Task::perform(async {}, |_| Message::FinalizeRecording);
                    }
                }

                if self.playback_status_rx.try_recv().is_ok() {
                    self.stop_playback_impl();
                    self.status_message = "Playback finished.".into();
                }
            }
            Message::Toggle => {
                if self.is_recording {
                    self.stop_recording_impl();
                } else {
                    self.start_recording_impl();
                }
            }
            Message::Reset => {}
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = if self.is_recording || self.is_playing || self.stopping_time.is_some() {
            time::every(Duration::from_millis(16)).map(Message::Tick)
        } else {
            Subscription::none()
        };

        let keyboard = keyboard::on_key_press(|key, _modifiers| match key {
            Key::Named(keyboard::key::Named::Space) => Some(Message::Toggle),
            Key::Character(ref c) if c == "p" => Some(Message::StopPlayback),
            _ => None,
        });

        Subscription::batch(vec![tick, keyboard])
    }

    fn view(&self) -> Element<'_, Message> {
        let secs = self.elapsed_time.as_secs();
        let cs = (self.elapsed_time.subsec_millis() / 10) as u64;
        let formatted = format!("{:02}:{:02}.{:02}", secs / 60, secs % 60, cs);

        let timer_text = text(formatted).size(40);

        let record_button =
            if !self.is_recording && !self.is_playing && self.stopping_time.is_none() {
                button(text("● Record")).on_press(Message::StartRecording)
            } else {
                button(text("■ Stop")).on_press(Message::StopRecording)
            };

        let stop_playback_button = if self.is_playing {
            button(text("■ Stop Playback")).on_press(Message::StopPlayback)
        } else {
            button(text("■ Stop Playback"))
        };

        let files_content = if self.files.is_empty() {
            column![text("No recordings found.")]
        } else {
            let mut files_col = column![];
            for file_name in &self.files {
                let row = row![
                    text(file_name).width(Length::Fill),
                    button(text("► Play")).on_press(Message::PlayFile(file_name.clone())),
                    button(text("✕ Delete")).on_press(Message::DeleteFile(file_name.clone())),
                ]
                .spacing(12);
                files_col = files_col.push(row);
            }
            files_col
        };

        let files_scroll = scrollable(files_content).height(Length::Fixed(220.0));

        let content = column![
            text("Voice Recorder").size(30),
            text(&self.status_message).size(16),
            timer_text,
            row![record_button, stop_playback_button].spacing(12),
            text("Recorded Files").size(22),
            files_scroll
        ]
        .spacing(16)
        .align_x(iced::Alignment::Center);

        center(content).into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

pub fn main() -> iced::Result {
    iced::application("Voice Recorder", VoiceRecorder::update, VoiceRecorder::view)
        .subscription(VoiceRecorder::subscription)
        .theme(VoiceRecorder::theme)
        .run()
}
