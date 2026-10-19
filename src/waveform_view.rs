//! Continuous, antialiased waveform silhouettes. Geometry is rebuilt only on
//! file/size/DPI changes; playback updates only the color clipping boundary.
use crate::waveform::Waveform;
use gpui::{
    BorderStyle, Bounds, ContentMask, Path, PathBuilder, Pixels, Window, fill, point, px, quad,
    rgb, rgba, size, transparent_black,
};
use std::sync::Arc;

#[derive(Default)]
pub struct WaveformCache {
    geometry: Option<Geometry>,
}

struct Geometry {
    wave: Arc<Waveform>,
    bounds: Bounds<Pixels>,
    scale: f32,
    fill: Path<Pixels>,
}

impl WaveformCache {
    pub fn paint(
        &mut self,
        wave: Option<&Arc<Waveform>>,
        bounds: Bounds<Pixels>,
        progress: Option<f32>,
        window: &mut Window,
    ) {
        let center = bounds.origin.y + bounds.size.height / 2.;
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, center),
                size(bounds.size.width, px(0.5)),
            ),
            rgb(0x26374d),
        ));
        if let Some(wave) = wave.filter(|w| !w.peaks.is_empty()) {
            let scale = window.scale_factor();
            if self.geometry.as_ref().is_none_or(|g| {
                !Arc::ptr_eq(&g.wave, wave) || g.bounds != bounds || g.scale != scale
            }) {
                self.geometry = Geometry::build(wave.clone(), bounds, scale);
            }
            if let Some(g) = &self.geometry {
                window.paint_path(g.fill.clone(), rgb(0x526d8e));
                if let Some(fraction) = progress.filter(|f| *f > 0.) {
                    let clip = Bounds::new(
                        bounds.origin,
                        size(bounds.size.width * fraction, bounds.size.height),
                    );
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        window.paint_path(g.fill.clone(), rgb(0x388cff));
                    });
                }
            }
        } else {
            self.geometry = None;
        }
        if let Some(fraction) = progress {
            let x = bounds.origin.x + bounds.size.width * fraction;
            let line_y = bounds.origin.y + px(8.);
            let line_h = bounds.size.height - px(16.);
            window.paint_quad(fill(
                Bounds::new(point(x - px(2.), line_y), size(px(5.), line_h)),
                rgba(0x6db9ff14),
            ));
            window.paint_quad(fill(
                Bounds::new(point(x, line_y), size(px(1.25), line_h)),
                rgb(0x9fceff),
            ));
            window.paint_quad(quad(
                Bounds::new(point(x - px(2.5), line_y - px(3.)), size(px(6.5), px(6.5))),
                px(3.25),
                rgb(0x9fceff),
                px(0.),
                transparent_black(),
                BorderStyle::Solid,
            ));
        }
    }
}

impl Geometry {
    fn build(wave: Arc<Waveform>, bounds: Bounds<Pixels>, scale: f32) -> Option<Self> {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 0. || height <= 0. {
            return None;
        }
        let count = ((width * scale).ceil() as usize).clamp(2, wave.peaks.len().max(2));
        let maximum = wave.peaks.iter().copied().fold(0., f32::max).max(0.001);
        let amplitudes: Vec<f32> = (0..count)
            .map(|i| {
                let start = (i * wave.peaks.len() / count).min(wave.peaks.len() - 1);
                let end = ((i + 1) * wave.peaks.len() / count)
                    .max(start + 1)
                    .min(wave.peaks.len());
                wave.peaks[start..end].iter().copied().fold(0., f32::max)
            })
            .collect();
        let center = bounds.origin.y + bounds.size.height / 2.;
        let outline: Vec<_> = amplitudes
            .iter()
            .enumerate()
            .map(|(i, value)| {
                // A small reconstruction filter removes bucket stair steps while
                // keeping real transients and the original timing intact.
                let left = amplitudes[i.saturating_sub(1)];
                let right = amplitudes[(i + 1).min(count - 1)];
                let amplitude = (left * 0.12 + value * 0.76 + right * 0.12) / maximum;
                let half_height = (amplitude.powf(0.72) * height * 0.41).max(0.35);
                (
                    bounds.origin.x + px(i as f32 * width / (count - 1) as f32),
                    px(half_height),
                )
            })
            .collect();
        let mut fill = PathBuilder::fill();
        for (i, &(x, half)) in outline.iter().enumerate() {
            let p = point(x, center - half);
            if i == 0 {
                fill.move_to(p);
            } else {
                fill.line_to(p);
            }
        }
        for &(x, half) in outline.iter().rev() {
            let p = point(x, center + half);
            fill.line_to(p);
        }
        fill.close();
        Some(Self {
            wave,
            bounds,
            scale,
            fill: fill.build().ok()?,
        })
    }
}
