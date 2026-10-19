//! Real decoder tests, plus an opt-in test that opens the actual output device.
#![allow(dead_code)]
#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/waveform.rs"]
mod waveform;

use audio::{Audio, Command, Snapshot};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("edolwave-test-{}.wav", std::process::id()));
        let count = 48_000u32 * 3;
        let mut bytes = Vec::with_capacity(count as usize * 2 + 44);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + count * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&96_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(count * 2).to_le_bytes());
        for i in 0..count {
            let sample = ((i as f32 * 440. * std::f32::consts::TAU / 48_000.).sin() * 300.) as i16;
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn decode_real_pcm_and_cancel_obsolete_analysis() {
    let f = Fixture::new();
    let generation = AtomicU64::new(7);
    let w = waveform::analyze(&f.0, &generation, 7).unwrap().unwrap();
    assert_eq!(w.duration, Duration::from_secs(3));
    assert_eq!(w.sample_rate, 48_000);
    assert_eq!(w.channels, 1);
    assert_eq!(w.peaks.len(), 2048);
    assert!(w.peaks.iter().all(|&v| v > 0. && v < 0.02));
    assert!(waveform::analyze(&f.0, &generation, 6).unwrap().is_none());
}

fn wait(audio: &Audio, description: &str, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let start = Instant::now();
    loop {
        let s = audio.snapshot();
        assert!(s.error.is_none(), "{description}: {:?}", s.error);
        if condition(&s) {
            return s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "timed out: {description}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "opens the audio output device; cargo test --test audio_flow -- --ignored --nocapture"]
fn real_output_play_pause_seek_repeat_and_format_matrix() {
    let fixture = Fixture::new();
    let player = Audio::new();
    player.send(Command::Volume(0.01));
    let mut files = vec![fixture.0.clone()];
    if let Some(dir) = std::env::var_os("EDOLWAVE_TEST_AUDIO_DIR") {
        files.extend(
            fs::read_dir(dir)
                .unwrap()
                .map(|p| p.unwrap().path())
                .filter(|p| p.is_file()),
        );
    }
    for path in files {
        let started = Instant::now();
        player.load(path.clone());
        let s = wait(&player, "load", |s| {
            s.path.as_ref() == Some(&path) && s.playing && s.waveform.is_some()
        });
        println!(
            "{}: {} ms, {}, {:?}",
            path.display(),
            started.elapsed().as_millis(),
            s.backend,
            s.duration
        );
        assert!(!s.duration.is_zero());
        player.send(Command::Toggle);
        wait(&player, "pause", |s| !s.playing);
        let seek_id = player.seek(s.duration / 2);
        wait(&player, "seek while paused", |new| {
            new.seek_revision == seek_id
                && new.position.abs_diff(s.duration / 2) < Duration::from_millis(100)
        });
        player.send(Command::Speed(1.5));
        player.send(Command::Volume(0.02));
        wait(&player, "speed and volume", |s| {
            s.speed == 1.5 && s.volume == 0.02
        });
        player.send(Command::Repeat);
        wait(&player, "repeat enabled", |s| s.repeat);
        player.seek(s.duration.saturating_sub(Duration::from_millis(80)));
        player.send(Command::Toggle);
        wait(&player, "loop restarted", |new| {
            new.playing && new.position < Duration::from_millis(300)
        });
        player.send(Command::Repeat);
        player.send(Command::Stop);
        wait(&player, "stop", |s| {
            !s.playing && s.position.is_zero() && !s.repeat
        });
        player.send(Command::Speed(1.));
    }
    // An unreadable file must not poison subsequent loads.
    player.load(PathBuf::from("nonexistent-edolwave-file.wav"));
    let start = Instant::now();
    while player.snapshot().error.is_none() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    player.load(fixture.0.clone());
    let start = Instant::now();
    while player.snapshot().error.is_some() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    wait(&player, "error recovery", |s| {
        s.path.as_ref() == Some(&fixture.0) && s.playing
    });
    drop(player);
    let prefix = format!("edolwave-{}-", std::process::id());
    assert!(
        !fs::read_dir(std::env::temp_dir()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(&prefix)
        }),
        "converted audio must be cleaned up when the player closes"
    );
}
