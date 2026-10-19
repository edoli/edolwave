use crate::audio::{Audio, Command, Snapshot};
use crate::waveform_view::WaveformCache;
use gpui::{
    BorderStyle, Bounds, Context, Div, ExternalPaths, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Render,
    ScrollWheelEvent, Stateful, Svg, Timer, Window, WindowControlArea, canvas, div, point,
    prelude::*, px, quad, rgb, size, svg, transparent_black,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

const BG: u32 = 0x141c27;
const PANEL: u32 = 0x1b2533;
const LINE: u32 = 0x2b384a;
const TEXT: u32 = 0xe9eff8;
const MUTED: u32 = 0x8c9cb1;
const BLUE: u32 = 0x388cff;
const SPEED_STEPS: [f32; 7] = [0.5, 0.75, 1., 1.25, 1.5, 1.75, 2.];

#[derive(Default)]
struct SeekPreview {
    pending: Option<(u64, Duration)>,
}

impl SeekPreview {
    fn begin(&mut self, request: u64, position: Duration) {
        self.pending = Some((request, position));
    }
    fn acknowledge(&mut self, completed: u64) {
        if self
            .pending
            .is_some_and(|(request, _)| request <= completed)
        {
            self.pending = None;
        }
    }
    fn position(&self, actual: Duration) -> Duration {
        self.pending.map_or(actual, |(_, target)| target)
    }
    fn clear(&mut self) {
        self.pending = None;
    }
}

pub struct PlayerView {
    audio: Audio,
    state: Snapshot,
    playlist: Vec<PathBuf>,
    index: usize,
    focus: FocusHandle,
    wave_bounds: Rc<Cell<Bounds<Pixels>>>,
    volume_bounds: Rc<Cell<Bounds<Pixels>>>,
    speed_bounds: Rc<Cell<Bounds<Pixels>>>,
    scrubbing: Option<f32>,
    seek_preview: SeekPreview,
    waveform_cache: Rc<RefCell<WaveformCache>>,
    volume_drag: bool,
    speed_menu_open: bool,
    volume_preview: Option<f32>,
    speed_preview: Option<f32>,
    last_volume: f32,
    settings: bool,
    started: Instant,
    first_render: bool,
}

impl PlayerView {
    pub fn new(
        files: Vec<PathBuf>,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let audio = Audio::new();
        let state = audio.snapshot();
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(33)).await;
                if this
                    .update(cx, |this, cx| {
                        let state = this.audio.snapshot();
                        if state.revision != this.state.revision {
                            this.seek_preview.acknowledge(state.seek_revision);
                            if this.volume_preview == Some(state.volume) {
                                this.volume_preview = None;
                            }
                            if this.speed_preview == Some(state.speed) {
                                this.speed_preview = None;
                            }
                            let ended = this.state.playing
                                && !state.playing
                                && !state.repeat
                                && !state.duration.is_zero()
                                && state.position >= state.duration
                                && state.error.is_none();
                            this.state = state;
                            if ended
                                && this.playlist.len() > 1
                                && this.index + 1 < this.playlist.len()
                            {
                                this.navigate(1, cx);
                            }
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut view = Self {
            audio,
            state,
            playlist: vec![],
            index: 0,
            focus,
            wave_bounds: Rc::new(Cell::new(Bounds::default())),
            volume_bounds: Rc::new(Cell::new(Bounds::default())),
            speed_bounds: Rc::new(Cell::new(Bounds::default())),
            scrubbing: None,
            seek_preview: SeekPreview::default(),
            waveform_cache: Rc::new(RefCell::new(WaveformCache::default())),
            volume_drag: false,
            speed_menu_open: false,
            volume_preview: None,
            speed_preview: None,
            last_volume: 0.8,
            settings: false,
            started,
            first_render: true,
        };
        if !files.is_empty() {
            view.open(files, cx);
        }
        view
    }

    fn open(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.playlist = files;
        self.index = 0;
        self.scrubbing = None;
        self.seek_preview.clear();
        if let Some(path) = self.playlist.first() {
            self.audio.load(path.clone());
            self.state.loading = true;
            self.state.path = Some(path.clone());
            self.state.error = None;
        }
        cx.notify();
    }

    fn choose(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("오디오 파일 열기".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ = this.update(cx, |this, cx| this.open(paths, cx));
            }
        })
        .detach();
    }

    fn navigate(&mut self, direction: isize, cx: &mut Context<Self>) {
        if self.playlist.len() > 1 {
            self.index =
                (self.index as isize + direction).rem_euclid(self.playlist.len() as isize) as usize;
            self.audio.load(self.playlist[self.index].clone());
            self.scrubbing = None;
            self.seek_preview.clear();
        } else {
            self.seek_to(Duration::ZERO, cx);
        }
        cx.notify();
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.path.is_none() {
            self.choose(window, cx);
        } else if !self.state.loading {
            self.audio.send(Command::Toggle);
        }
    }

    fn fraction(&self, x: Pixels) -> f32 {
        let b = self.wave_bounds.get();
        if b.size.width <= px(0.) {
            return 0.;
        }
        ((x - b.origin.x) / b.size.width).clamp(0., 1.)
    }

    fn finish_seek(&mut self, cx: &mut Context<Self>) {
        if let Some(fraction) = self.scrubbing.take() {
            self.seek_to(self.state.duration.mul_f32(fraction), cx);
        }
    }

    fn seek_to(&mut self, position: Duration, cx: &mut Context<Self>) {
        let position = if self.state.duration.is_zero() {
            position
        } else {
            position.min(self.state.duration.saturating_sub(Duration::from_millis(1)))
        };
        self.seek_preview.begin(self.audio.seek(position), position);
        cx.notify();
    }

    fn displayed_position(&self) -> Duration {
        self.scrubbing
            .map(|f| self.state.duration.mul_f32(f))
            .unwrap_or_else(|| self.seek_preview.position(self.state.position))
    }

    fn displayed_volume(&self) -> f32 {
        self.volume_preview.unwrap_or(self.state.volume)
    }

    fn set_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        let volume = volume.clamp(0., 1.);
        self.volume_preview = Some(volume);
        self.audio.send(Command::Volume(volume));
        cx.notify();
    }

    fn set_volume_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
        let bounds = self.volume_bounds.get();
        if bounds.size.width > px(0.) {
            self.set_volume((x - bounds.origin.x) / bounds.size.width, cx);
        }
    }

    fn set_speed(&mut self, speed: f32, cx: &mut Context<Self>) {
        if speed != self.speed_preview.unwrap_or(self.state.speed) {
            self.speed_preview = Some(speed);
            self.audio.send(Command::Speed(speed));
            cx.notify();
        }
    }

    fn scroll_speed(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(16.)).y;
        if delta != px(0.) {
            let speed = self.speed_preview.unwrap_or(self.state.speed);
            let index = SPEED_STEPS
                .iter()
                .position(|&step| step == speed)
                .unwrap_or(2);
            let next = if delta > px(0.) {
                (index + 1).min(SPEED_STEPS.len() - 1)
            } else {
                index.saturating_sub(1)
            };
            self.set_speed(SPEED_STEPS[next], cx);
        }
        cx.stop_propagation();
    }

    fn speed_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = self.speed_bounds.clone();
        div()
            .id("speed-dropdown")
            .relative()
            .w(px(80.))
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(if self.speed_menu_open { BLUE } else { LINE }))
            .bg(rgb(PANEL))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(0x26364b)))
            .child(
                canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .size_full()
                    .top_0()
                    .left_0(),
            )
            .child(format!(
                "{:.2}×",
                self.speed_preview.unwrap_or(self.state.speed)
            ))
            .child(icon("chevron", 12., MUTED))
            .on_click(cx.listener(|this, _, _, cx| {
                this.speed_menu_open = !this.speed_menu_open;
                this.settings = false;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_speed(event, cx)))
    }

    fn speed_menu(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.speed_bounds.get();
        let viewport = window.viewport_size();
        let selected = self.speed_preview.unwrap_or(self.state.speed);
        div()
            .id("speed-menu")
            .occlude()
            .absolute()
            .left(bounds.origin.x.min(viewport.width - px(104.)).max(px(8.)))
            .bottom(viewport.height - bounds.origin.y + px(4.))
            .w(px(96.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(7.))
            .bg(rgb(0x202c3d))
            .border_1()
            .border_color(rgb(0x3b4d65))
            .shadow_lg()
            .children(SPEED_STEPS.into_iter().enumerate().map(|(index, speed)| {
                div()
                    .id(("speed-option", index))
                    .h(px(24.))
                    .flex_shrink_0()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(speed == selected, |el| el.bg(rgb(BLUE)))
                    .hover(|el| el.bg(rgb(0x36577d)))
                    .child(format!("{speed:.2}×"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_speed(speed, cx);
                        this.speed_menu_open = false;
                        cx.notify();
                    }))
            }))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_speed(event, cx)))
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if !this.speed_bounds.get().contains(&event.position) {
                    this.speed_menu_open = false;
                    cx.notify();
                }
            }))
    }

    fn controls(&self, narrow: bool, cx: &mut Context<Self>) -> Div {
        let transport = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .child(
                transport("previous", "previous", false)
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(-1, cx))),
            )
            .child(
                transport(
                    "play",
                    if self.state.playing { "pause" } else { "play" },
                    true,
                )
                .on_click(cx.listener(|this, _, w, cx| this.play(w, cx))),
            )
            .child(
                transport("next", "next", false)
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(1, cx))),
            )
            .child(
                transport("stop", "stop", false).on_click(cx.listener(|this, _, _, cx| {
                    this.seek_preview.clear();
                    this.audio.send(Command::Stop);
                    cx.notify();
                })),
            );
        let volume = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .child(
                tool(
                    "mute",
                    if self.displayed_volume() == 0. {
                        "mute"
                    } else {
                        "volume"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.mute(cx))),
            )
            .child(self.volume(cx))
            .child(
                div()
                    .w(px(28.))
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "{}%",
                        (self.displayed_volume() * 100.).round() as u32
                    )),
            );
        let speed = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("재생 속도"),
            )
            .child(self.speed_dropdown(cx));
        let repeat = div()
            .id("repeat")
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .px_2()
            .py_1()
            .rounded_md()
            .hover(|s| s.bg(rgb(PANEL)))
            .child(icon(
                "repeat",
                20.,
                if self.state.repeat { BLUE } else { MUTED },
            ))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("반복 재생"),
            )
            .child(
                div()
                    .w(px(32.))
                    .h(px(19.))
                    .rounded_full()
                    .bg(rgb(if self.state.repeat { BLUE } else { LINE }))
                    .p(px(3.))
                    .flex()
                    .when(self.state.repeat, |d| d.justify_end())
                    .child(div().size(px(13.)).rounded_full().bg(rgb(TEXT))),
            )
            .on_click(cx.listener(|this, _, _, _| this.audio.send(Command::Repeat)));
        div()
            .h(px(if narrow { 72. } else { 48. }))
            .flex_shrink_0()
            .px(px(16.))
            .flex()
            .map(|el| {
                if narrow {
                    el.flex_col()
                        .gap(px(8.))
                        .child(
                            div()
                                .h(px(32.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(transport)
                                .child(volume),
                        )
                        .child(
                            div()
                                .h(px(32.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(speed)
                                .child(repeat),
                        )
                } else {
                    el.items_center()
                        .justify_between()
                        .gap(px(4.))
                        .child(transport)
                        .child(volume)
                        .child(speed)
                        .child(repeat)
                }
            })
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        if e.keystroke.modifiers.control && key == "o" {
            self.choose(window, cx);
            return;
        }
        if e.keystroke.modifiers.control && key == "q" {
            cx.quit();
            return;
        }
        match key {
            "space" => self.play(window, cx),
            "left" => self.seek_to(
                self.displayed_position()
                    .saturating_sub(Duration::from_secs(5)),
                cx,
            ),
            "right" => self.seek_to(
                self.displayed_position()
                    .saturating_add(Duration::from_secs(5)),
                cx,
            ),
            "home" => self.seek_to(Duration::ZERO, cx),
            "up" => self.set_volume(self.displayed_volume() + 0.05, cx),
            "down" => self.set_volume(self.displayed_volume() - 0.05, cx),
            "r" => self.audio.send(Command::Repeat),
            "m" => self.mute(cx),
            "escape" => {
                self.settings = false;
                self.speed_menu_open = false;
                self.scrubbing = None;
                cx.notify();
            }
            _ => {}
        }
    }

    fn mute(&mut self, cx: &mut Context<Self>) {
        let volume = self.displayed_volume();
        let v = if volume > 0. {
            self.last_volume = volume;
            0.
        } else {
            self.last_volume.max(0.05)
        };
        self.set_volume(v, cx);
    }

    fn waveform(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let wave = self.state.waveform.clone();
        let fraction = if self.state.duration.is_zero() {
            0.
        } else {
            (self.displayed_position().as_secs_f64() / self.state.duration.as_secs_f64())
                .clamp(0., 1.) as f32
        };
        let bounds_cell = self.wave_bounds.clone();
        let cache = self.waveform_cache.clone();
        let has_file = self.state.path.is_some();
        div()
            .id("waveform")
            .relative()
            .w_full()
            .flex_1()
            .min_h(px(48.))
            .cursor_crosshair()
            .child(
                canvas(
                    move |b, _, _| {
                        bounds_cell.set(b);
                    },
                    move |bounds, _, window, _| {
                        cache.borrow_mut().paint(
                            wave.as_ref(),
                            bounds,
                            has_file.then_some(fraction),
                            window,
                        );
                    },
                )
                .size_full(),
            )
            .when(self.state.analyzing || self.state.loading, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_2()
                        .right_0()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(if self.state.loading {
                            "파일 여는 중…"
                        } else {
                            "파형 분석 중…"
                        }),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if !this.state.duration.is_zero() && !this.state.loading {
                        this.scrubbing = Some(this.fraction(e.position.x));
                        cx.notify();
                    }
                }),
            )
    }

    fn volume(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cell = self.volume_bounds.clone();
        let v = self.displayed_volume();
        div()
            .id("volume-bar")
            .w(px(80.))
            .h(px(30.))
            .cursor_pointer()
            .child(
                canvas(
                    move |b, _, _| cell.set(b),
                    move |b, _, window, _| {
                        let y = b.origin.y + b.size.height / 2. - px(2.);
                        window.paint_quad(quad(
                            Bounds::new(point(b.origin.x, y), size(b.size.width, px(4.))),
                            px(2.),
                            rgb(LINE),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        window.paint_quad(quad(
                            Bounds::new(point(b.origin.x, y), size(b.size.width * v, px(4.))),
                            px(2.),
                            rgb(BLUE),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        window.paint_quad(quad(
                            Bounds::new(
                                point(b.origin.x + b.size.width * v - px(5.), y - px(3.)),
                                size(px(10.), px(10.)),
                            ),
                            px(5.),
                            rgb(TEXT),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.volume_drag = true;
                    this.set_volume_at(e.position.x, cx);
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.)).y;
                if delta != px(0.) {
                    let step = if delta > px(0.) { 0.05 } else { -0.05 };
                    this.set_volume(this.displayed_volume() + step, cx);
                }
                cx.stop_propagation();
            }))
    }
}

impl Render for PlayerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.first_render {
            self.first_render = false;
            if let Some(path) = std::env::var_os("EDOLWAVE_STARTUP_LOG") {
                let _ = std::fs::write(
                    path,
                    format!(
                        "first_render_ms={:.2}\n",
                        self.started.elapsed().as_secs_f64() * 1000.
                    ),
                );
            }
        }
        let name = self
            .state
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "EdolWave".into());
        let pos = self.displayed_position();
        let metadata = self
            .state
            .waveform
            .as_ref()
            .map(|w| {
                format!(
                    "{} Hz  ·  {}  ·  {}",
                    w.sample_rate,
                    if w.channels == 1 {
                        "Mono".into()
                    } else {
                        format!("{} ch", w.channels)
                    },
                    self.state.backend
                )
            })
            .unwrap_or_else(|| "NATIVE AUDIO PLAYER".into());
        let narrow = window.viewport_size().width < px(640.);
        div().id("player").size_full().relative().flex().flex_col()
            .bg(rgb(BG)).text_color(rgb(TEXT)).font_family("Segoe UI").text_size(px(14.))
            .on_key_down(cx.listener(Self::key))
            .on_drop(cx.listener(|this, files: &ExternalPaths, _, cx| this.open(files.paths().to_vec(), cx)))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if !e.pressed_button.is_some_and(|b| b == MouseButton::Left) {
                    if this.scrubbing.is_some() { this.finish_seek(cx); }
                    this.volume_drag = false; return;
                }
                if this.scrubbing.is_some() { this.scrubbing = Some(this.fraction(e.position.x)); cx.notify(); }
                if this.volume_drag { this.set_volume_at(e.position.x, cx); }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| { this.finish_seek(cx); this.volume_drag = false; }))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, _, _, cx| { this.finish_seek(cx); this.volume_drag = false; }))
            .child(div().flex().flex_shrink_0().items_center().h(px(44.)).px(px(16.)).gap(px(14.))
                .child(div().id("title-drag").flex_1().min_w_0().h_full().flex().items_center().gap(px(14.)).cursor_default()
                    .window_control_area(WindowControlArea::Drag)
                    .child(icon("wave", 26., 0x49caff))
                    .child(div().min_w_0().flex_1().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_ellipsis().child(name)))
                .child(tool("open", "folder").on_click(cx.listener(|this, _, window, cx| this.choose(window, cx))))
                .child(tool("settings", "settings").on_click(cx.listener(|this, _, _, cx| { this.settings = !this.settings; this.speed_menu_open = false; cx.notify(); })))
                .child(div().w(px(1.)).h(px(20.)).bg(rgb(LINE)).mx_1())
                .child(tool("minimize", "minimize").on_click(|_, w, _| w.minimize_window()))
                .child(tool("maximize", "maximize").on_click(|_, w, _| w.zoom_window()))
                .child(tool("close", "close").hover(|s| s.bg(rgb(0x663142))).on_click(|_, _, cx| cx.quit())))
            .child(
                div().track_focus(&self.focus).flex_1().min_h_0().px(px(16.)).flex().flex_col()
                    .child(self.waveform(cx))
                    .child(
                        div().h(px(20.)).flex_shrink_0().flex().items_center().justify_between().gap(px(12.))
                            .child(
                                div().flex().flex_shrink_0().items_center().gap(px(8.)).text_size(px(12.)).line_height(px(16.)).font_weight(FontWeight::SEMIBOLD)
                                    .child(div().text_color(rgb(BLUE)).child(time(pos)))
                                    .child(div().text_color(rgb(0x65758c)).child("/"))
                                    .child(div().text_color(rgb(MUTED)).child(time(self.state.duration)))
                            )
                            .child(div().min_w_0().text_ellipsis().text_size(px(10.)).text_color(rgb(0x65758c)).child(metadata))
                    )
            )
            .child(self.controls(narrow, cx))
            .when_some(self.state.error.clone(), |el, error| el.child(div().h(px(24.)).flex_shrink_0().px(px(28.)).text_size(px(11.)).text_color(rgb(0xffa9a9)).text_ellipsis().child(error)))
            .when(self.settings, |el| el.child(div().id("help-panel").absolute().top(px(48.)).right(px(20.)).w(px(306.)).max_h(px(260.)).overflow_y_scroll().rounded_lg().bg(rgb(0x202c3d)).border_1().border_color(rgb(0x3b4d65)).shadow_lg()
                .child(div().p_5().flex().flex_col().gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("EdolWave · 빠른 사용 안내"))
                .children([("파일 열기", "Ctrl + O"), ("재생 / 일시정지", "Space"), ("5초 탐색", "← / →"), ("처음으로", "Home"), ("볼륨", "↑ / ↓"), ("음소거 / 반복", "M / R")].into_iter().map(|(a,b)| div().flex().justify_between().text_size(px(12.)).child(a).child(div().text_color(rgb(MUTED)).child(b))))
                .child(div().border_t_1().border_color(rgb(LINE)).pt_3().text_size(px(11.)).text_color(rgb(MUTED)).child("여러 파일을 함께 열면 이전/다음 버튼으로 이동합니다. 배속은 드롭다운에서 선택합니다. 볼륨과 배속 위에서 휠로 조절할 수 있습니다. 배속에 따라 음높이도 바뀝니다."))
                .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child("Opus · WMA · APE 등은 설치된 FFmpeg를 자동으로 사용합니다.")))))
            .when(self.speed_menu_open, |el| el.child(self.speed_menu(window, cx)))
    }
}

fn icon(name: &'static str, size: f32, color: u32) -> Svg {
    svg().path(name).size(px(size)).text_color(rgb(color))
}
fn tool(id: &'static str, name: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x263345)))
        .child(icon(name, 19., MUTED))
}
fn transport(id: &'static str, name: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(32.))
        .h(px(24.))
        .flex_shrink_0()
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(if primary { 0x62a7ff } else { LINE }))
        .bg(rgb(if primary { BLUE } else { PANEL }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(if primary { 0x55a0ff } else { 0x26364b })))
        .child(icon(name, 18., TEXT))
}
fn time(d: Duration) -> String {
    let ms = d.as_millis();
    let mins = ms / 60000;
    if mins >= 60 {
        format!(
            "{}:{:02}:{:02}.{:03}",
            mins / 60,
            mins % 60,
            ms / 1000 % 60,
            ms % 1000
        )
    } else {
        format!("{:02}:{:02}.{:03}", mins, ms / 1000 % 60, ms % 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_handles_subsecond_and_long_files() {
        assert_eq!(time(Duration::from_millis(178)), "00:00.178");
        assert_eq!(time(Duration::from_millis(3_661_843)), "1:01:01.843");
    }

    #[test]
    fn seek_preview_rejects_old_playback_frames_until_completion() {
        let mut preview = SeekPreview::default();
        let target = Duration::from_secs(8);
        preview.begin(7, target);
        // The worker can publish another frame at the original position before
        // processing the seek. Releasing the mouse must not expose that frame.
        preview.acknowledge(6);
        assert_eq!(preview.position(Duration::from_secs(1)), target);
        assert_eq!(preview.position(Duration::from_millis(1020)), target);
        preview.acknowledge(7);
        assert_eq!(
            preview.position(Duration::from_millis(8020)),
            Duration::from_millis(8020)
        );
    }

    #[test]
    fn an_older_seek_completion_cannot_clear_a_newer_preview() {
        let mut preview = SeekPreview::default();
        preview.begin(8, Duration::from_secs(2));
        preview.begin(9, Duration::from_secs(6));
        preview.acknowledge(8);
        assert_eq!(
            preview.position(Duration::from_secs(2)),
            Duration::from_secs(6)
        );
        preview.acknowledge(9);
        assert_eq!(
            preview.position(Duration::from_millis(6010)),
            Duration::from_millis(6010)
        );
    }
}
