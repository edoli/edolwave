//! Streaming WSOLA: pitch-preserving tempo changes with bounded look-ahead.
//!
//! Original implementation: waveform correlation chooses a shared offset for
//! every channel, followed by complementary overlap-add windows. All positions
//! are in source frames, not interleaved samples or stretched playback seconds.
use rodio::{ChannelCount, SampleRate, Source, source::UniformSourceIterator};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub struct PlaybackControl {
    speed: Arc<AtomicU32>,
    position_us: Arc<AtomicU64>,
}

impl PlaybackControl {
    fn new(speed: f32) -> Self {
        let control = Self {
            speed: Arc::new(AtomicU32::new(1_f32.to_bits())),
            position_us: Arc::new(AtomicU64::new(0)),
        };
        control.set_speed(speed);
        control
    }

    pub fn set_speed(&self, speed: f32) {
        if speed.is_finite() {
            self.speed
                .store(speed.clamp(0.5, 2.).to_bits(), Ordering::Relaxed);
        }
    }

    pub fn position(&self) -> Duration {
        Duration::from_micros(self.position_us.load(Ordering::Relaxed))
    }

    fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }
}

// Some decoders (notably Vorbis) initially expose an empty decoded packet.
// UniformSourceIterator otherwise takes that empty span without advancing the
// decoder, and reports a false EOF. Allow one channel frame to advance it.
struct NonEmptySpans<S>(S);
impl<S: Source> Iterator for NonEmptySpans<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.0.next()
    }
}
impl<S: Source> Source for NonEmptySpans<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.0
            .current_span_len()
            .map(|n| n.max(self.0.channels().get() as usize))
    }
    fn channels(&self) -> ChannelCount {
        self.0.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.0.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.0.total_duration()
    }
}

pub struct Wsola<S: Source> {
    input: UniformSourceIterator<NonEmptySpans<S>>,
    channels: ChannelCount,
    rate: SampleRate,
    control: PlaybackControl,
    // Sliding interleaved input buffer; absolute frame index of its first frame.
    samples: Vec<f32>,
    origin: usize,
    eof: bool,
    hop: usize,
    search: usize,
    window: Vec<f32>,
    tail: Vec<f32>,
    output: Vec<f32>,
    output_index: usize,
    output_len: usize,
    previous_start: Option<usize>,
    media_frame: f64,
    block_speed: f64,
}

impl<S: Source> Wsola<S> {
    pub fn new(input: S, speed: f32) -> (Self, PlaybackControl) {
        let channels = input.channels();
        let rate = input.sample_rate();
        let hop = (rate.get() as usize / 100).max(1); // 10 ms, 20 ms windows.
        let search = (rate.get() as usize * 8 / 1000).max(1);
        let control = PlaybackControl::new(speed);
        let mut source = Self {
            input: UniformSourceIterator::new(NonEmptySpans(input), channels, rate),
            channels,
            rate,
            control: control.clone(),
            samples: Vec::with_capacity((6 * hop + 4 * search) * channels.get() as usize),
            origin: 0,
            eof: false,
            hop,
            search,
            window: Vec::with_capacity(hop),
            tail: vec![0.; hop * channels.get() as usize],
            output: vec![0.; hop * channels.get() as usize],
            output_index: 0,
            output_len: 0,
            previous_start: None,
            media_frame: 0.,
            block_speed: 1.,
        };
        // Prime on the audio worker, before handing the source to the mixer.
        // File look-ahead is decoded immediately; no leading silence is added.
        source.read_until(2 * hop + search);
        let length = source.input_end();
        if source.eof && length < 4 * hop {
            // Tiny UI clicks must still have several overlap steps, including
            // both boundaries. A one-frame source is repeated/truncated safely.
            source.hop = (length / 8).max(1);
            source.search = search.min(source.hop);
        }
        source.window.extend((0..source.hop).map(|i| {
            if source.hop == 1 {
                1.
            } else {
                0.5 - 0.5 * (std::f32::consts::PI * i as f32 / (source.hop - 1) as f32).cos()
            }
        }));
        source.fill_block();
        (source, control)
    }

    fn channel_count(&self) -> usize {
        self.channels.get() as usize
    }

    fn input_end(&self) -> usize {
        self.origin + self.samples.len() / self.channel_count()
    }

    fn read_until(&mut self, end: usize) {
        let ch = self.channel_count();
        while !self.eof && self.input_end() < end {
            let start = self.samples.len();
            for c in 0..ch {
                if let Some(value) = self.input.next() {
                    self.samples
                        .push(if value.is_finite() { value } else { 0. });
                } else {
                    self.eof = true;
                    // Preserve a final partial channel frame without inventing
                    // a whole extra frame for the ordinary, aligned EOF case.
                    if c > 0 {
                        self.samples.resize(start + ch, 0.);
                    }
                    break;
                }
            }
        }
    }

    fn sample(&self, frame: usize, channel: usize) -> f32 {
        if frame < self.origin {
            return 0.;
        }
        self.samples
            .get((frame - self.origin) * self.channel_count() + channel)
            .copied()
            .unwrap_or(0.)
    }

    fn correlation(&self, start: usize, stride: usize, expected: usize) -> f32 {
        let ch = self.channel_count();
        let offset = (start - self.origin) * ch;
        let candidate = &self.samples[offset..offset + self.hop * ch];
        let mut dot = 0.;
        let mut energy_a = 0.;
        let mut energy_b = 0.;
        // Sum channels separately rather than downmixing: anti-phase stereo
        // must not cancel the matching signal or shift its stereo image.
        for (a, b) in candidate
            .chunks_exact(ch)
            .step_by(stride)
            .zip(self.tail.chunks_exact(ch).take(self.hop).step_by(stride))
        {
            for (&a, &b) in a.iter().zip(b) {
                dot += a * b;
                energy_a += a * a;
                energy_b += b * b;
            }
        }
        let similarity = dot / (energy_a * energy_b).sqrt().max(1e-20);
        // Prefer nearby equivalents on periodic tones and silence, limiting
        // jitter without allowing amplitude changes to bias waveform matching.
        similarity - 0.001 * start.abs_diff(expected) as f32 / self.search as f32
    }

    fn best_start(&self, expected: usize) -> usize {
        let lo = expected.saturating_sub(self.search).max(self.origin);
        let hi = (expected + self.search).min(self.input_end().saturating_sub(self.hop));
        if hi < lo {
            return expected.min(self.input_end()).max(self.origin);
        }
        // At unity tempo a natural continuation needs neither a search nor a
        // crossfade. This also makes every 1x sample bit-identical to the input.
        if self.block_speed == 1.
            && let Some(natural) = self.previous_start.map(|p| p + self.hop)
            && (lo..=hi).contains(&natural)
        {
            return natural;
        }
        let mut best = expected.clamp(lo, hi);
        let mut score = self.correlation(best, 2, expected);
        // Coarse-to-fine search keeps 48 kHz stereo comfortably inexpensive.
        // The final candidates use all samples to avoid a coarse-only match.
        for start in (lo..=hi).step_by(4) {
            let next = self.correlation(start, 2, expected);
            if next > score {
                score = next;
                best = start;
            }
        }
        let coarse = best;
        score = self.correlation(best, 1, expected);
        for start in coarse.saturating_sub(3).max(lo)..=(coarse + 3).min(hi) {
            let next = self.correlation(start, 1, expected);
            if next > score {
                score = next;
                best = start;
            }
        }
        best
    }

    fn fill_block(&mut self) -> bool {
        self.block_speed = self.control.speed() as f64;
        let expected = self.media_frame.round() as usize;
        self.read_until(expected + self.search + 2 * self.hop);
        let end = self.input_end();
        if self.eof && self.media_frame >= end as f64 {
            return false;
        }
        let frames = if self.eof {
            (((end as f64 - self.media_frame) / self.block_speed).ceil() as usize).min(self.hop)
        } else {
            self.hop
        };
        if frames == 0 {
            return false;
        }
        let last = self.eof && self.media_frame + frames as f64 * self.block_speed >= end as f64;
        let first = self.previous_start.is_none();
        let start = if first {
            0
        } else if last {
            // Anchor the final crossfade to the real end, preserving short
            // attacks/tails rather than silently dropping an incomplete window.
            end.saturating_sub(frames).max(self.origin)
        } else {
            self.best_start(expected)
        };
        let ch = self.channel_count();
        for i in 0..frames {
            let weight = if frames == 1 {
                1.
            } else {
                self.window[i * (self.hop - 1) / (frames - 1)]
            };
            for c in 0..ch {
                let a = self.tail[i * ch + c];
                let b = self.sample(start + i, c);
                self.output[i * ch + c] = if first || a == b || weight == 1. {
                    b
                } else if weight == 0. {
                    a
                } else {
                    a + (b - a) * weight
                };
            }
        }
        for i in 0..self.hop {
            for c in 0..ch {
                self.tail[i * ch + c] = self.sample(start + self.hop + i, c);
            }
        }
        self.previous_start = Some(start);
        self.output_index = 0;
        self.output_len = frames * ch;
        let next = self.media_frame + frames as f64 * self.block_speed;
        let keep = (next.floor() as usize)
            .saturating_sub(self.search + 2 * self.hop)
            .max(self.origin)
            .min(end);
        let discard = (keep - self.origin) * ch;
        self.samples.copy_within(discard.., 0);
        self.samples.truncate(self.samples.len() - discard);
        self.origin = keep;
        true
    }
}

impl<S: Source> Iterator for Wsola<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.output_index == self.output_len && !self.fill_block() {
            return None;
        }
        let value = self.output[self.output_index];
        self.output_index += 1;
        if self.output_index.is_multiple_of(self.channel_count()) {
            self.media_frame += self.block_speed;
            if self.eof {
                self.media_frame = self.media_frame.min(self.input_end() as f64);
            }
            self.control.position_us.store(
                (self.media_frame * 1_000_000. / self.rate.get() as f64) as u64,
                Ordering::Relaxed,
            );
        }
        Some(value)
    }
}

impl<S: Source> Source for Wsola<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    // Tempo can change at any point, so stretched wall-clock duration is unknown.
    // The worker retains the original duration and recreates this source on seek.
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::time::Instant;

    fn noise(frames: usize, channels: usize) -> Vec<f32> {
        let mut seed = 91_u32;
        (0..frames * channels)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as f64 / u32::MAX as f64 - 0.5) as f32
            })
            .collect()
    }

    fn source(
        samples: Vec<f32>,
        rate: u32,
        channels: u16,
        speed: f32,
    ) -> (Wsola<SamplesBuffer>, PlaybackControl) {
        Wsola::new(
            SamplesBuffer::new(
                channels.try_into().unwrap(),
                rate.try_into().unwrap(),
                samples,
            ),
            speed,
        )
    }

    #[test]
    fn an_empty_initial_decoder_span_is_not_end_of_stream() {
        struct InitiallyEmpty {
            inner: SamplesBuffer,
            advanced: bool,
        }
        impl Iterator for InitiallyEmpty {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                self.advanced = true;
                self.inner.next()
            }
        }
        impl Source for InitiallyEmpty {
            fn current_span_len(&self) -> Option<usize> {
                if self.advanced {
                    self.inner.current_span_len()
                } else {
                    Some(0)
                }
            }
            fn channels(&self) -> ChannelCount {
                self.inner.channels()
            }
            fn sample_rate(&self) -> SampleRate {
                self.inner.sample_rate()
            }
            fn total_duration(&self) -> Option<Duration> {
                self.inner.total_duration()
            }
        }
        let input = noise(10001, 2);
        let inner = SamplesBuffer::new(
            2.try_into().unwrap(),
            48000.try_into().unwrap(),
            input.clone(),
        );
        let (processor, _) = Wsola::new(
            InitiallyEmpty {
                inner,
                advanced: false,
            },
            1.,
        );
        assert_eq!(processor.collect::<Vec<_>>(), input);
    }

    #[test]
    fn unity_is_bit_exact_including_tiny_files_and_partial_windows() {
        for channels in [1, 2, 6] {
            for frames in [0, 1, 2, 7, 23, 239, 1439, 1440, 4800, 10001] {
                let input = noise(frames, channels);
                let (processor, control) = source(input.clone(), 44100, channels as u16, 1.);
                assert_eq!(
                    processor.collect::<Vec<_>>(),
                    input,
                    "{frames} frames, {channels} channels"
                );
                assert!(
                    control
                        .position()
                        .abs_diff(Duration::from_secs_f64(frames as f64 / 44100.))
                        <= Duration::from_micros(1)
                );
            }
        }
    }

    #[test]
    fn output_duration_and_boundaries_are_preserved_at_all_ui_speeds() {
        for speed in [0.5, 0.75, 1., 1.25, 1.5, 1.75, 2.] {
            for frames in [0, 1, 2, 7, 23, 239, 1440, 2401, 4800] {
                let input = noise(frames, 2);
                let (processor, control) = source(input.clone(), 48000, 2, speed);
                let output: Vec<_> = processor.collect();
                assert_eq!(
                    output.len(),
                    (frames as f64 / speed as f64).ceil() as usize * 2,
                    "speed={speed}, frames={frames}"
                );
                assert!(output.iter().all(|v| v.is_finite() && v.abs() <= 0.50001));
                if frames > 0 {
                    assert_eq!(&output[..2], &input[..2]);
                }
                if frames >= 7 {
                    assert_eq!(
                        &output[output.len() - 2..],
                        &input[input.len() - 2..],
                        "tail at {speed}x, {frames} frames"
                    );
                }
                assert!(
                    control
                        .position()
                        .abs_diff(Duration::from_secs_f64(frames as f64 / 48000.))
                        <= Duration::from_micros(1)
                );
            }
        }
    }

    fn tone(hz: f32, frames: usize, rate: u32) -> Vec<f32> {
        (0..frames)
            .map(|i| (i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect()
    }

    fn measured_pitch(samples: &[f32], rate: u32) -> f64 {
        // Interpolated positive zero crossings, excluding both edge windows.
        let trim = rate as usize / 20;
        let middle = &samples[trim..samples.len() - trim];
        let mut crossings = Vec::new();
        for (i, pair) in middle.windows(2).enumerate() {
            if pair[0] <= 0. && pair[1] > 0. {
                crossings.push(i as f64 - pair[0] as f64 / (pair[1] - pair[0]) as f64);
            }
        }
        rate as f64 * (crossings.len() - 1) as f64 / (crossings.last().unwrap() - crossings[0])
    }

    #[test]
    fn pitch_stays_constant_for_bass_voice_and_high_tones() {
        for (rate, hz) in [(24000, 440.), (44100, 80.), (48000, 4000.)] {
            for speed in [0.5, 0.75, 1.25, 1.5, 2.] {
                let (processor, _) = source(tone(hz, rate as usize, rate), rate, 1, speed);
                let output: Vec<_> = processor.collect();
                let measured = measured_pitch(&output, rate);
                assert!(
                    (measured - hz as f64).abs() / (hz as f64) < 0.015,
                    "pitch changed: {hz} Hz -> {measured:.2} Hz at {speed}x, {rate} Hz rate"
                );
                let rms = (output.iter().map(|v| v * v).sum::<f32>() / output.len() as f32).sqrt();
                assert!(
                    rms > 0.25,
                    "pitch test must not pass with near-silent output: {rms}"
                );
            }
        }
    }

    #[test]
    fn stereo_matching_preserves_phase_and_independent_channels() {
        let mono = tone(330., 12000, 24000);
        let input: Vec<_> = mono.into_iter().flat_map(|v| [v, -v]).collect();
        for speed in [0.5, 1.5, 2.] {
            let (processor, _) = source(input.clone(), 24000, 2, speed);
            let output: Vec<_> = processor.collect();
            assert!(output.chunks_exact(2).all(|p| (p[0] + p[1]).abs() < 1e-7));
            assert!(output.iter().any(|v| v.abs() > 0.4));
        }
    }

    #[test]
    fn live_speed_changes_keep_a_monotonic_source_clock_and_bounded_memory() {
        let (mut processor, control) = source(tone(440., 24000 * 2, 24000), 24000, 1, 1.);
        let capacity = processor.samples.capacity();
        let mut previous = Duration::ZERO;
        let mut max_frames = 0;
        let mut emitted = 0;
        while processor.next().is_some() {
            emitted += 1;
            if emitted == 2400 {
                control.set_speed(2.);
            }
            if emitted == 4800 {
                control.set_speed(0.5);
            }
            if emitted == 9600 {
                control.set_speed(1.5);
            }
            let position = control.position();
            assert!(position >= previous);
            previous = position;
            max_frames = max_frames.max(processor.samples.len());
            assert_eq!(
                processor.samples.capacity(),
                capacity,
                "input buffer reallocated during playback"
            );
        }
        assert!(max_frames <= capacity);
        assert_eq!(control.position(), Duration::from_secs(2));
        // A fresh source after a seek starts with no stale clock or overlap.
        let (mut after_seek, after_control) = source(vec![0.25; 240], 24000, 1, 2.);
        assert_eq!(after_control.position(), Duration::ZERO);
        assert_eq!(after_seek.next(), Some(0.25));
        assert_eq!(after_control.position(), Duration::from_micros(83));
    }

    #[test]
    #[ignore = "manual release DSP throughput measurement"]
    fn benchmark_stereo_throughput() {
        let rate = 48000;
        let input: Vec<_> = tone(440., rate * 5, rate as u32)
            .into_iter()
            .flat_map(|v| [v, -v])
            .collect();
        for speed in [0.5, 0.75, 1., 1.5, 2.] {
            let started = Instant::now();
            let (processor, _) = source(input.clone(), rate as u32, 2, speed);
            let samples = processor.map(std::hint::black_box).count();
            let elapsed = started.elapsed().as_secs_f64();
            let seconds = samples as f64 / (rate * 2) as f64;
            println!(
                "WSOLA {speed:.2}x: {elapsed:.4}s processing time / {seconds:.3}s output ({:.3}% realtime)",
                elapsed / seconds * 100.
            );
        }
    }
}
