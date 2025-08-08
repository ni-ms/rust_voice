use eframe::{
    egui::{self},
    epaint::vec2,
};
use std::fs;
use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Stream, StreamConfig};
use eframe::egui::{Color32, Shadow, Stroke};
use hound::{self, WavReader, WavSpec};

fn write_wav_file(path: &str, spec: WavSpec, samples: &[f32]) -> io::Result<()> {
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

struct VoiceRecorderApp {
    is_recording: Arc<Mutex<bool>>,
    audio_data: Arc<Mutex<Vec<f32>>>,
    status_message: String,
    input_stream: Option<Stream>,
    output_stream: Option<Stream>,
    files: Vec<String>,
}

impl Default for VoiceRecorderApp {
    fn default() -> Self {
        let mut app = Self {
            is_recording: Arc::new(Mutex::new(false)),
            audio_data: Arc::new(Mutex::new(Vec::new())),
            status_message: "Ready to record.".to_string(),
            input_stream: None,
            output_stream: None,
            files: Vec::new(),
        };

        app.update_file_list();
        app
    }
}

impl VoiceRecorderApp {
    fn update_file_list(&mut self) {
        self.files.clear();
        if let Ok(entries) = fs::read_dir(".") {
            for entry in entries.flatten() {
                if let Some(file_name) = entry.file_name().to_str() {
                    if file_name.ends_with(".wav") {
                        self.files.push(file_name.to_string());
                    }
                }
            }
        }
    }

    fn start_recording(&mut self) {
        let mut is_recording_lock = self.is_recording.lock().unwrap();

        if !*is_recording_lock {
            self.audio_data.lock().unwrap().clear();

            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .expect("Failed to find default input device");
            let config = device
                .default_input_config()
                .expect("Failed to get default input config");

            let audio_data_callback = Arc::clone(&self.audio_data);

            let stream = device
                .build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        audio_data_callback.lock().unwrap().extend_from_slice(data);
                    },
                    |err| eprintln!("An error occurred on the audio stream: {}", err),
                    None,
                )
                .unwrap();

            stream.play().unwrap();

            self.input_stream = Some(stream);
            *is_recording_lock = true;
            self.status_message = "Recording...".to_string();
        }
    }

    fn stop_recording(&mut self) {
        let was_recording = {
            let mut is_recording = self.is_recording.lock().unwrap();
            if *is_recording {
                *is_recording = false;
                true
            } else {
                false
            }
        };

        if !was_recording {
            return;
        }

        self.input_stream = None;

        let filename = format!("recording_{}.wav", self.files.len() + 1);

        let samples: Vec<f32> = {
            let mut audio_data = self.audio_data.lock().unwrap();

            std::mem::take(&mut *audio_data)
        };

        if samples.is_empty() {
            self.status_message = "Error saving file: No audio data captured".to_string();
            return;
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .expect("Failed to find default input device");
        let config = device
            .default_input_config()
            .expect("Failed to get default input config");

        let spec = WavSpec {
            channels: config.channels() as u16,
            sample_rate: config.sample_rate().0,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };

        match write_wav_file(&filename, spec, &samples) {
            Ok(()) => {
                self.status_message = format!("Recording stopped. File saved as '{}'", filename);

                self.update_file_list();
            }
            Err(e) => {
                self.status_message = format!("Error saving file: {}", e);
            }
        }
    }

    fn play_file(&mut self, filename: &str) {
        self.output_stream = None;

        let mut reader = match WavReader::open(filename) {
            Ok(r) => r,
            Err(e) => {
                self.status_message = format!("Error opening file: {}", e);
                return;
            }
        };

        let spec = reader.spec();
        let samples: Vec<f32> = reader.samples().filter_map(Result::ok).collect();
        let samples_arc = Arc::new(Mutex::new(samples));

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .expect("Failed to find default output device");

        let supported_configs = device
            .supported_output_configs()
            .expect("error querying supported output configs");

        let supported_config = supported_configs
            .filter(|c| c.channels() == spec.channels as u16)
            .min_by_key(|c| (c.max_sample_rate().0 as i64 - spec.sample_rate as i64).abs())
            .expect("No supported config found");

        let config: StreamConfig = supported_config
            .with_sample_rate(supported_config.max_sample_rate())
            .into();

        let (tx, rx) = mpsc::channel();
        let samples_callback = Arc::clone(&samples_arc);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut samples_lock = samples_callback.lock().unwrap();
                    let len = data.len().min(samples_lock.len());
                    data[..len].copy_from_slice(&samples_lock[..len]);
                    samples_lock.drain(..len);

                    if samples_lock.is_empty() {
                        let _ = tx.send(());
                    }
                },
                |err| eprintln!("An error occurred on the audio stream: {}", err),
                None,
            )
            .unwrap();

        stream.play().unwrap();
        self.output_stream = Some(stream);
        self.status_message = format!("Playing: {}", filename);

        let _ = rx.recv();
        self.output_stream = None;
        self.status_message = "Playback finished.".to_string();
    }
}

impl eframe::App for VoiceRecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));

        let visuals = egui::Visuals {
            dark_mode: true,
            override_text_color: Some(Color32::from_rgb(244, 247, 245)),
            ..Default::default()
        };
        ctx.set_visuals(visuals);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(Color32::from_rgb(8, 9, 10))
                    .inner_margin(egui::Margin::same(12.0 as i8)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading(
                        egui::RichText::new("Voice Recorder")
                            .size(30.0)
                            .strong()
                            .color(Color32::from_rgb(244, 247, 245)),
                    );
                    ui.label(
                        egui::RichText::new(&self.status_message)
                            .size(16.0)
                            .color(Color32::from_rgb(167, 162, 169)),
                    );
                });

                ui.add_space(20.0);

                // Recording Controls
                egui::Frame::default()
                    .fill(Color32::from_rgb(34, 40, 35)) // #222823
                    .rounding(egui::Rounding::same(10.0 as u8))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(87, 90, 94))) // #575A5E
                    .inner_margin(egui::Margin::same(16.0 as i8))
                    .show(ui, |ui| {
                        let is_recording = *self.is_recording.lock().unwrap();

                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!is_recording, |ui| {
                                if ui
                                    .add_sized(
                                        [120.0, 40.0],
                                        egui::Button::new("⏺ Record")
                                            .fill(Color32::from_rgb(220, 20, 60)), // Crimson touch
                                    )
                                    .clicked()
                                {
                                    self.start_recording();
                                }
                            });

                            ui.add_enabled_ui(is_recording, |ui| {
                                if ui
                                    .add_sized(
                                        [120.0, 40.0],
                                        egui::Button::new("⏹ Stop")
                                            .fill(Color32::from_rgb(87, 90, 94)), 
                                    )
                                    .clicked()
                                {
                                    self.stop_recording();
                                }
                            });
                        });
                    });

                ui.add_space(24.0);
                ui.heading(
                    egui::RichText::new("📁 Recorded Files")
                        .size(22.0)
                        .color(Color32::from_rgb(244, 247, 245)),
                );
                ui.add_space(10.0);

                // File list
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        let mut file_to_play: Option<String> = None;

                        if self.files.is_empty() {
                            ui.label(
                                egui::RichText::new("No recordings found.")
                                    .color(Color32::from_rgb(167, 162, 169)), // #A7A2A9
                            );
                        } else {
                            for file_name in &self.files {
                                egui::Frame::default()
                                    .fill(Color32::from_rgb(34, 40, 35)) // #222823
                                    .rounding(egui::Rounding::same(8.0 as u8))
                                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(87, 90, 94))) // #575A5E
                                    .inner_margin(egui::Margin::same(12.0 as i8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(file_name)
                                                    .size(16.0)
                                                    .color(Color32::from_rgb(244, 247, 245)),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui
                                                        .add(
                                                            egui::Button::new("▶ Play").fill(
                                                                Color32::from_rgb(0, 180, 120),
                                                            ), // Nice warm green
                                                        )
                                                        .clicked()
                                                    {
                                                        file_to_play = Some(file_name.clone());
                                                    }
                                                },
                                            );
                                        });
                                    });

                                ui.add_space(6.0);
                            }
                        }

                        if let Some(file_name) = file_to_play {
                            self.play_file(&file_name);
                        }
                    });
            });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(vec2(400.0, 400.0)),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Voice Recorder",
        options,
        Box::new(|_cc| Ok(Box::new(VoiceRecorderApp::default()))),
    )
}
