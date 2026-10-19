//! Bounded-memory peak extraction. No decoded PCM is retained.
use anyhow::Result;
use rodio::{Decoder, Source};
use std::{
    fs::File,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Waveform {
    pub peaks: Vec<f32>,
    pub duration: Duration,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Coarsens completed buckets in pairs, retaining an exact partial final bucket.
struct Peaks {
    values: Vec<f32>,
    stride: usize,
    pending: usize,
    peak: f32,
    samples: u64,
}

impl Peaks {
    fn new() -> Self {
        Self {
            values: Vec::with_capacity(8192),
            stride: 16,
            pending: 0,
            peak: 0.,
            samples: 0,
        }
    }

    fn push(&mut self, sample: f32) {
        self.peak = self
            .peak
            .max(if sample.is_finite() { sample.abs() } else { 0. });
        self.pending += 1;
        self.samples += 1;
        if self.pending == self.stride {
            self.values.push(self.peak);
            self.pending = 0;
            self.peak = 0.;
            if self.values.len() == 8192 {
                for i in 0..4096 {
                    self.values[i] = self.values[i * 2].max(self.values[i * 2 + 1]);
                }
                self.values.truncate(4096);
                self.stride *= 2;
            }
        }
    }

    fn finish(mut self, bins: usize) -> Vec<f32> {
        if self.pending > 0 {
            self.values.push(self.peak);
        }
        if self.samples == 0 {
            return vec![];
        }
        // Map using actual sample offsets, so the partial bucket cannot distort time.
        (0..bins)
            .map(|i| {
                let start = ((i as u64 * self.samples / bins as u64) / self.stride as u64) as usize;
                let end = (((i + 1) as u64 * self.samples / bins as u64).saturating_sub(1)
                    / self.stride as u64) as usize;
                self.values
                    [start.min(self.values.len() - 1)..=end.max(start).min(self.values.len() - 1)]
                    .iter()
                    .copied()
                    .fold(0., f32::max)
                    .min(1.)
            })
            .collect()
    }
}

pub fn analyze(path: &Path, generation: &AtomicU64, id: u64) -> Result<Option<Waveform>> {
    let decoder = Decoder::try_from(File::open(path)?)?;
    let sample_rate = decoder.sample_rate().get();
    let channels = decoder.channels().get();
    let mut peaks = Peaks::new();
    for (i, sample) in decoder.enumerate() {
        if i % 16384 == 0 && generation.load(Ordering::Relaxed) != id {
            return Ok(None);
        }
        peaks.push(sample);
    }
    let duration =
        Duration::from_secs_f64(peaks.samples as f64 / sample_rate as f64 / channels as f64);
    Ok(Some(Waveform {
        peaks: peaks.finish(2048),
        duration,
        sample_rate,
        channels,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarsening_preserves_transient_and_time() {
        let mut p = Peaks::new();
        let count = 256 * 8192 + 31;
        for i in 0..count {
            p.push(if i == count - 1 { 0.9 } else { 0.1 });
        }
        assert!(p.values.len() <= 8192);
        assert_eq!(p.samples, count as u64);
        let bins = p.finish(128);
        assert_eq!(bins.len(), 128);
        assert_eq!(bins[127], 0.9);
        assert_eq!(bins[0], 0.1);
    }

    #[test]
    fn handles_empty_short_and_nonfinite_samples() {
        assert!(Peaks::new().finish(8).is_empty());
        let mut p = Peaks::new();
        for v in [f32::NAN, f32::INFINITY, -0.5] {
            p.push(v);
        }
        assert_eq!(p.finish(8), vec![0.5; 8]);
    }
}
