//! Audio device, disk I/O and seeks live off the UI thread.
use crate::waveform::{self, Waveform};
use anyhow::{Context as _, Result, anyhow};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use std::{
    fs::File,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    /// The last seek command that has finished, including failed seeks.
    pub seek_revision: u64,
    pub path: Option<PathBuf>,
    pub duration: Duration,
    pub position: Duration,
    pub playing: bool,
    pub loading: bool,
    pub analyzing: bool,
    pub volume: f32,
    pub speed: f32,
    pub repeat: bool,
    pub waveform: Option<Arc<Waveform>>,
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            seek_revision: 0,
            path: None,
            duration: Duration::ZERO,
            position: Duration::ZERO,
            playing: false,
            loading: false,
            analyzing: false,
            volume: 0.8,
            speed: 1.,
            repeat: false,
            waveform: None,
            error: None,
        }
    }
}

pub enum Command {
    Shutdown,
    Load(PathBuf, u64),
    Toggle,
    Stop,
    Seek(Duration, u64),
    Volume(f32),
    Speed(f32),
    Repeat,
    Waveform(u64, Result<Option<Waveform>, String>),
}

pub struct Audio {
    tx: Sender<Command>,
    pub state: Arc<Mutex<Snapshot>>,
    generation: Arc<AtomicU64>,
    worker: Option<thread::JoinHandle<()>>,
    next_seek: AtomicU64,
}

impl Audio {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let generation = Arc::new(AtomicU64::new(0));
        let (s, g, sender) = (state.clone(), generation.clone(), tx.clone());
        let worker = thread::Builder::new()
            .name("audio".into())
            .spawn(move || Worker::new(s, g, sender).run(rx))
            .expect("audio worker");
        Self {
            tx,
            state,
            generation,
            worker: Some(worker),
            next_seek: AtomicU64::new(0),
        }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
    pub fn load(&self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let id = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Command::Load(path, id));
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn seek(&self, position: Duration) -> u64 {
        let id = self.next_seek.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Command::Seek(position, id));
        id
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.generation.store(u64::MAX, Ordering::Relaxed);
        let _ = self.tx.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker {
    state: Arc<Mutex<Snapshot>>,
    generation: Arc<AtomicU64>,
    tx: Sender<Command>,
    output: Option<MixerDeviceSink>,
    player: Option<Player>,
    source: Option<PathBuf>,
    id: u64,
    position_offset: Duration,
    analyses: Vec<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(state: Arc<Mutex<Snapshot>>, generation: Arc<AtomicU64>, tx: Sender<Command>) -> Self {
        Self {
            state,
            generation,
            tx,
            output: None,
            player: None,
            source: None,
            id: 0,
            position_offset: Duration::ZERO,
            analyses: vec![],
        }
    }

    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut s = self.state.lock().unwrap();
        f(&mut s);
        s.revision += 1;
    }

    fn run(mut self, rx: Receiver<Command>) {
        while self.generation.load(Ordering::Relaxed) != u64::MAX {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(cmd) => {
                    let seek_id = match &cmd {
                        Command::Seek(_, id) => Some(*id),
                        _ => None,
                    };
                    let result = self.handle(cmd);
                    if seek_id.is_some() || result.is_err() {
                        self.update(|s| {
                            if let Some(id) = seek_id {
                                s.seek_revision = id;
                            }
                            if let Err(err) = result
                                && self.id == self.generation.load(Ordering::Relaxed)
                            {
                                s.error = Some(format!("{err:#}"));
                                s.loading = false;
                            }
                        });
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Some(player) = &self.player {
                let s = self.state.lock().unwrap().clone();
                if s.playing {
                    if player.empty() {
                        if s.repeat {
                            if let Err(e) = self.restart(true, Duration::ZERO) {
                                self.update(|s| {
                                    s.playing = false;
                                    s.error = Some(e.to_string());
                                });
                            }
                        } else {
                            self.update(|s| {
                                s.playing = false;
                                s.position = s.duration;
                            });
                        }
                    } else {
                        let pos = player.get_pos().saturating_add(self.position_offset);
                        self.update(|s| {
                            s.position = if s.duration.is_zero() {
                                pos
                            } else {
                                pos.min(s.duration)
                            }
                        });
                    }
                }
            }
        }
    }

    fn handle(&mut self, cmd: Command) -> Result<()> {
        match cmd {
            Command::Shutdown => {}
            Command::Load(path, id) => {
                if id != self.generation.load(Ordering::Relaxed) {
                    return Ok(());
                }
                self.id = id;
                self.player.take();
                self.source.take();
                self.update(|s| {
                    s.path = Some(path.clone());
                    s.loading = true;
                    s.playing = false;
                    s.waveform = None;
                    s.analyzing = false;
                    s.error = None;
                    s.position = Duration::ZERO;
                    s.duration = Duration::ZERO;
                });
                Decoder::try_from(File::open(&path)?)
                    .context("지원하지 않는 오디오 형식이거나 손상된 파일입니다")?;
                if id != self.generation.load(Ordering::Relaxed) {
                    return Ok(());
                }
                self.source = Some(path.clone());
                self.restart(true, Duration::ZERO)?;
                self.update(|s| {
                    s.loading = false;
                    s.analyzing = true;
                });
                let (tx, generation) = (self.tx.clone(), self.generation.clone());
                self.analyses.retain(|task| !task.is_finished());
                let task = thread::Builder::new()
                    .name("waveform".into())
                    .spawn(move || {
                        let result =
                            waveform::analyze(&path, &generation, id).map_err(|e| e.to_string());
                        let _ = tx.send(Command::Waveform(id, result));
                    })?;
                self.analyses.push(task);
            }
            Command::Toggle => {
                if let Some(p) = &self.player {
                    let playing = self.state.lock().unwrap().playing;
                    if playing {
                        p.pause();
                        self.update(|s| s.playing = false);
                    } else if p.empty() {
                        self.restart(true, Duration::ZERO)?;
                    } else {
                        p.play();
                        self.update(|s| s.playing = true);
                    }
                }
            }
            Command::Stop => {
                if self.source.is_some() {
                    self.restart(false, Duration::ZERO)?;
                }
            }
            Command::Seek(pos, _) => {
                if self.source.is_some() {
                    let s = self.state.lock().unwrap().clone();
                    let pos = if s.duration.is_zero() {
                        pos
                    } else {
                        pos.min(s.duration.saturating_sub(Duration::from_millis(1)))
                    };
                    // Prepare the new decoder here, away from the real-time output callback.
                    self.restart(s.playing, pos)?;
                }
            }
            Command::Volume(v) => {
                let v = v.clamp(0., 1.);
                if let Some(p) = &self.player {
                    p.set_volume(v);
                }
                self.update(|s| s.volume = v);
            }
            Command::Speed(v) => {
                if let Some(p) = &self.player {
                    p.set_speed(v);
                }
                self.update(|s| s.speed = v);
            }
            Command::Repeat => self.update(|s| s.repeat = !s.repeat),
            Command::Waveform(id, result) => {
                if id == self.id && id == self.generation.load(Ordering::Relaxed) {
                    self.update(|s| {
                        s.analyzing = false;
                        match result {
                            Ok(Some(w)) => {
                                s.duration = w.duration;
                                s.waveform = Some(Arc::new(w));
                            }
                            Ok(None) => {}
                            Err(e) => s.error = Some(format!("파형 분석: {e}")),
                        }
                    });
                }
            }
        }
        Ok(())
    }

    fn restart(&mut self, playing: bool, pos: Duration) -> Result<()> {
        if self.output.is_none() {
            // Request ~10 ms; use a supported device configuration if unavailable.
            self.output = Some(
                DeviceSinkBuilder::from_default_device()?
                    .with_buffer_size(rodio::cpal::BufferSize::Fixed(512))
                    .open_sink_or_fallback()?,
            );
        }
        let mut source = Decoder::try_from(File::open(
            self.source.as_ref().context("파일이 없습니다")?,
        )?)?;
        let duration = source.total_duration().unwrap_or_default();
        if !pos.is_zero() && source.try_seek(pos).is_err() {
            // Some demuxers reject valid seeks near EOF (notably raw AAC).
            // Decode forward from a clean reader as a universal, bounded-memory fallback.
            source = Decoder::try_from(File::open(self.source.as_ref().unwrap())?)?;
            let samples = (pos.as_secs_f64()
                * source.sample_rate().get() as f64
                * source.channels().get() as f64) as u64;
            for i in 0..samples {
                if i % 16384 == 0 && self.generation.load(Ordering::Relaxed) != self.id {
                    return Err(anyhow!("탐색이 취소되었습니다"));
                }
                if source.next().is_none() {
                    break;
                }
            }
        }
        let s = self.state.lock().unwrap().clone();
        let player = Player::connect_new(self.output.as_ref().unwrap().mixer());
        player.pause();
        player.set_volume(s.volume);
        player.set_speed(s.speed);
        player.append(source);
        self.player = Some(player);
        self.position_offset = pos;
        if playing {
            self.player.as_ref().unwrap().play();
        }
        self.update(|s| {
            s.playing = playing;
            s.position = pos;
            if s.waveform.is_none() && !duration.is_zero() {
                s.duration = duration;
            }
        });
        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.generation.store(u64::MAX, Ordering::Relaxed);
        self.player.take();
        self.output.take();
        for task in self.analyses.drain(..) {
            let _ = task.join();
        }
    }
}
