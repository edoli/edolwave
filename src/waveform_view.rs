//! Continuous, antialiased waveform silhouettes. Geometry is rebuilt only on
//! file/size/DPI changes; playback updates only the color clipping boundary.
use crate::theme::Palette;
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
        colors: Palette,
        window: &mut Window,
    ) {
        let scale = window.scale_factor();
        let snap = |value: Pixels| px((f32::from(value) * scale).round() / scale);
        // Fractional quad edges can cover alternating numbers of device pixels
        // as the playhead moves, making its thickness and brightness flicker.
        // Share one snapped boundary with the played-waveform clipping mask.
        let playhead_x =
            progress.map(|fraction| snap(bounds.origin.x + bounds.size.width * fraction));
        let center = bounds.origin.y + bounds.size.height / 2.;
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, center),
                size(bounds.size.width, px(0.5)),
            ),
            rgb(colors.wave_axis),
        ));
        if let Some(wave) = wave.filter(|w| !w.peaks.is_empty()) {
            if self.geometry.as_ref().is_none_or(|g| {
                !Arc::ptr_eq(&g.wave, wave) || g.bounds != bounds || g.scale != scale
            }) {
                self.geometry = Geometry::build(wave.clone(), bounds, scale);
            }
            if let Some(g) = &self.geometry {
                window.paint_path(g.fill.clone(), rgb(colors.wave_idle));
                if let Some(x) = playhead_x.filter(|x| *x > bounds.origin.x) {
                    let clip =
                        Bounds::new(bounds.origin, size(x - bounds.origin.x, bounds.size.height));
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        window.paint_path(g.fill.clone(), rgb(colors.accent));
                    });
                }
            }
        } else {
            self.geometry = None;
        }
        if let Some(x) = playhead_x {
            let line_y = snap(bounds.origin.y + px(8.));
            let line_h = snap(bounds.bottom() - px(8.)) - line_y;
            let line_width = px((1.25 * scale).round().max(1.) / scale);
            window.paint_quad(fill(
                Bounds::new(point(x - snap(px(2.)), line_y), size(snap(px(5.)), line_h)),
                rgba(colors.playhead_glow),
            ));
            window.paint_quad(fill(
                Bounds::new(point(x, line_y), size(line_width, line_h)),
                rgb(colors.playhead),
            ));
            window.paint_quad(quad(
                Bounds::new(
                    point(
                        x + line_width / 2. - snap(px(6.5)) / 2.,
                        line_y - snap(px(3.)),
                    ),
                    size(snap(px(6.5)), snap(px(6.5))),
                ),
                px(3.25),
                rgb(colors.playhead),
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
