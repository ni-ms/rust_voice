Rust Voice Recorder v{{VERSION}}
===============================

A lightweight, privacy-focused voice recorder written in Rust.

Features
--------
- Record audio from the default microphone.
- Play back recordings instantly.
- Rename or delete recordings through the interface.
- All data is stored locally; no network access required.

System Requirements
-------------------
- Windows 10/11 (64-bit).
- No installation needed; just run the executable.

Usage
-----
1. Download and extract the ZIP package.
2. Run `rust_voice.exe`.
3. Click **Record** (or press Space) to start/stop recording.
4. Select a file and click **Play** to listen.

Technical Details
-----------------
- Audio format: 48 kHz WAV.
- Built with Rust, CPAL (audio), Iced (GUI) and Hound (WAV).
- Executable size: ~5 MB.
- The binary is signed with a self-signed certificate to reduce security warnings.

Troubleshooting
---------------
- No input detected: check microphone permissions in Windows Settings.
- No playback: verify your audio output device and volume.

Contributing
------------
Pull requests and bug reports are welcome at the project repository.

License
-------
This project is licensed under the MIT License.
