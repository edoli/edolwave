use crate::audio::{Audio, Command, Snapshot};
use crate::settings::{Settings, Store};
use crate::theme::{Palette, Theme};
use crate::update::{self, CheckState, InstallState};
use crate::waveform_view::WaveformCache;
use gpui::{
    BorderStyle, Bounds, Context, Div, ExternalPaths, FocusHandle, FontWeight, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, PathPromptOptions, Pixels, Render,
    ScrollWheelEvent, Stateful, Svg, Timer, Window, WindowControlArea, canvas, div, point,
    prelude::*, px, quad, rgb, rgba, size, svg, transparent_black,
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

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
    volume_feedback_until: Option<Instant>,
    speed_preview: Option<f32>,
    last_volume: f32,
    info_open: bool,
    settings_open: bool,
    settings_store: Store,
    preferences: Settings,
    settings_error: Option<String>,
    palette: Palette,
    update_service: update::Service,
    update_events: async_channel::Sender<update::Event>,
    update_check: CheckState,
    update_install: InstallState,
    started: Instant,
    first_render: bool,
}

impl PlayerView {
    pub fn new(
        files: Vec<PathBuf>,
        settings_store: Store,
        preferences: Settings,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_appearance(window, |this, window, cx| {
            this.palette = this.preferences.theme.palette(window.appearance());
            cx.notify();
        })
        .detach();
        let palette = preferences.theme.palette(window.appearance());
        let audio = Audio::new();
        // Queue preferences before any file is loaded, so playback starts with
        // the saved values even while the audio worker is still starting.
        audio.send(Command::Volume(preferences.volume));
        audio.send(Command::Speed(preferences.speed));
        if preferences.repeat {
            audio.send(Command::Repeat);
        }
        let state = Snapshot {
            volume: preferences.volume,
            speed: preferences.speed,
            repeat: preferences.repeat,
            ..audio.snapshot()
        };
        let focus = cx.focus_handle();
        window.focus(&focus);
        let (update_events, receiver) = async_channel::bounded(32);
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                if this
                    .update(cx, |this, cx| {
                        match event {
                            update::Event::Checked(result) => {
                                this.update_check = match result {
                                    Ok(Some(release)) => CheckState::Available(release),
                                    Ok(None) => CheckState::Current,
                                    Err(error) => CheckState::Failed(error),
                                };
                            }
                            update::Event::Progress { received, total } => {
                                this.update_install = InstallState::Downloading { received, total };
                            }
                            update::Event::Downloaded(result) => {
                                this.update_install = match result {
                                    Ok(package) => InstallState::Ready(package),
                                    Err(error) => InstallState::Failed(error),
                                };
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(33)).await;
                if this
                    .update(cx, |this, cx| {
                        if !this.volume_drag
                            && this
                                .volume_feedback_until
                                .is_some_and(|until| Instant::now() >= until)
                        {
                            this.volume_feedback_until = None;
                            cx.notify();
                        }
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
            volume_feedback_until: None,
            speed_preview: None,
            last_volume: if preferences.volume > 0. {
                preferences.volume
            } else {
                0.8
            },
            info_open: false,
            settings_open: false,
            settings_store,
            preferences,
            settings_error: None,
            palette,
            update_service: update::Service::new(),
            update_events,
            update_check: CheckState::Checking,
            update_install: InstallState::Idle,
            started,
            first_render: true,
        };
        if !files.is_empty() {
            view.open(files, cx);
        }
        view.update_service
            .check_in_background(view.update_events.clone());
        view
    }

    fn check_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update_check, CheckState::Checking)
            || matches!(self.update_install, InstallState::Downloading { .. })
        {
            return;
        }
        self.update_install = InstallState::Idle;
        self.update_check = CheckState::Checking;
        self.update_service
            .check_in_background(self.update_events.clone());
        cx.notify();
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        let CheckState::Available(release) = &self.update_check else {
            return;
        };
        if !matches!(
            self.update_install,
            InstallState::Idle | InstallState::Failed(_)
        ) {
            return;
        }
        self.update_install = InstallState::Downloading {
            received: 0,
            total: 0,
        };
        self.update_service
            .download_in_background(release.clone(), self.update_events.clone());
        cx.notify();
    }

    fn install_update(&mut self, cx: &mut Context<Self>) {
        let state = std::mem::replace(&mut self.update_install, InstallState::Idle);
        let InstallState::Ready(package) = state else {
            self.update_install = state;
            return;
        };
        match package.launch() {
            Ok(true) => cx.quit(),
            Ok(false) => self.update_install = InstallState::Opened,
            Err(error) => self.update_install = InstallState::Failed(error.to_string()),
        }
        cx.notify();
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

    pub fn open_from_instance(
        &mut self,
        files: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !files.is_empty() {
            self.open(files, cx);
        }
        self.info_open = false;
        self.settings_open = false;
        self.speed_menu_open = false;
        window.focus(&self.focus);
        window.activate_window();
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
        self.volume_feedback_until = Some(Instant::now() + Duration::from_millis(1200));
        self.audio.send(Command::Volume(volume));
        self.preferences.volume = volume;
        if !self.volume_drag {
            self.persist(|settings| settings.volume = volume);
        }
        cx.notify();
    }

    fn finish_volume_drag(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.volume_drag) {
            let volume = self.preferences.volume;
            self.persist(|settings| settings.volume = volume);
            self.volume_feedback_until = Some(Instant::now() + Duration::from_millis(1200));
            cx.notify();
        }
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
            self.preferences.speed = speed;
            self.persist(|settings| settings.speed = speed);
            cx.notify();
        }
    }

    fn persist(&mut self, change: impl FnOnce(&mut Settings)) {
        self.settings_error = self.settings_store.update(change).err().map(|error| {
            eprintln!("설정을 저장하지 못했습니다: {error:#}");
            "설정을 저장하지 못했습니다. 설정 폴더의 접근 권한을 확인해 주세요.".into()
        });
    }

    fn toggle_repeat(&mut self, cx: &mut Context<Self>) {
        self.preferences.repeat = !self.preferences.repeat;
        let repeat = self.preferences.repeat;
        self.audio.send(Command::Repeat);
        self.persist(|settings| settings.repeat = repeat);
        cx.notify();
    }

    fn set_theme(&mut self, theme: Theme, window: &Window, cx: &mut Context<Self>) {
        self.preferences.theme = theme;
        self.palette = theme.palette(window.appearance());
        self.persist(|settings| settings.theme = theme);
        cx.notify();
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
        let colors = self.palette;
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
            .border_color(rgb(if self.speed_menu_open {
                colors.accent
            } else {
                colors.line
            }))
            .bg(rgb(colors.panel))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(move |s| s.bg(rgb(colors.hover)))
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
            .child(icon("chevron", 12., colors.muted))
            .tooltip(move |_, cx| cx.new(|_| TextTooltip("재생 속도".into(), colors)).into())
            .on_click(cx.listener(|this, _, _, cx| {
                this.speed_menu_open = !this.speed_menu_open;
                this.info_open = false;
                this.settings_open = false;
                cx.notify();
            }))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.scroll_speed(event, cx)))
    }

    fn speed_menu(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.palette;
        let bounds = self.speed_bounds.get();
        let viewport = window.viewport_size();
        let selected = self.speed_preview.unwrap_or(self.state.speed);
        div()
            .id("speed-menu")
            .occlude()
            .absolute()
            .left(
                bounds
                    .origin
                    .x
                    .min(viewport.width - bounds.size.width - px(8.))
                    .max(px(8.)),
            )
            .bottom(viewport.height - bounds.origin.y + px(4.))
            .w(bounds.size.width)
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(7.))
            .bg(rgb(colors.menu))
            .border_1()
            .border_color(rgb(colors.menu_border))
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
                    .when(speed == selected, |el| {
                        el.bg(rgb(colors.accent)).text_color(rgb(colors.on_accent))
                    })
                    .hover(move |el| {
                        el.bg(rgb(if speed == selected {
                            colors.selected_menu_hover
                        } else {
                            colors.menu_hover
                        }))
                    })
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

    fn settings_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.palette;
        let (label, color, enabled, hint) = match &self.update_check {
            CheckState::Checking => (
                "확인 중…".to_owned(),
                colors.muted,
                false,
                "최신 버전을 확인하고 있습니다.".to_owned(),
            ),
            CheckState::Current => (
                "최신 버전".to_owned(),
                colors.success,
                false,
                "현재 최신 버전을 사용하고 있습니다.".to_owned(),
            ),
            CheckState::Failed(error) => {
                ("확인 실패".to_owned(), colors.warning, false, error.clone())
            }
            CheckState::Available(release) => {
                let (label, enabled, hint) = match &self.update_install {
                    InstallState::Idle => (
                        format!("v{} 업데이트", release.version),
                        true,
                        update::install_hint().to_owned(),
                    ),
                    InstallState::Downloading { received, total } => {
                        let percent = if *total == 0 {
                            0.
                        } else {
                            *received as f64 / *total as f64 * 100.
                        };
                        (
                            format!("다운로드 {:.0}%", percent),
                            false,
                            "설치 파일을 다운로드하고 검증하고 있습니다.".to_owned(),
                        )
                    }
                    InstallState::Ready(_) => (
                        if cfg!(windows) {
                            "설치하고 종료"
                        } else {
                            "설치 파일 열기"
                        }
                        .to_owned(),
                        true,
                        format!("파일 검증 완료. {}", update::install_hint()),
                    ),
                    InstallState::Failed(error) => {
                        ("다시 다운로드".to_owned(), true, error.clone())
                    }
                    InstallState::Opened => (
                        "다시 다운로드".to_owned(),
                        true,
                        format!("설치 파일을 열었습니다. {}", update::install_hint()),
                    ),
                };
                (label, colors.accent, enabled, hint)
            }
        };
        let ready = matches!(self.update_install, InstallState::Ready(_));
        let action = if matches!(self.update_check, CheckState::Available(_)) {
            settings_button(colors, "install-update", label, enabled)
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |this, _, _, cx| {
                        if ready {
                            this.install_update(cx);
                        } else {
                            if matches!(this.update_install, InstallState::Opened) {
                                this.update_install = InstallState::Idle;
                            }
                            this.download_update(cx);
                        }
                    }))
                })
                .into_any_element()
        } else {
            div()
                .text_size(px(12.))
                .text_color(rgb(color))
                .child(label)
                .into_any_element()
        };
        let body = div()
            .id("settings-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(14.))
            .py(px(8.))
            .child(
                div()
                    .id("update-row")
                    .h(px(36.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(format!("현재 버전: v{}", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(
                        div()
                            .id("update-status")
                            .flex_shrink_0()
                            .tooltip(move |_, cx| {
                                cx.new(|_| TextTooltip(hint.clone(), colors)).into()
                            })
                            .child(action),
                    ),
            )
            .child(
                div()
                    .id("theme-row")
                    .py(px(8.))
                    .child(div().mb(px(6.)).text_size(px(12.)).child("테마"))
                    .child(
                        div().flex().gap(px(4.)).children(
                            [
                                (Theme::Light, "밝은 테마"),
                                (Theme::Dark, "다크 테마"),
                                (Theme::System, "시스템 설정"),
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(index, (theme, label))| {
                                let selected = self.preferences.theme == theme;
                                div()
                                    .id(("theme-option", index))
                                    .flex_1()
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(rgb(if selected {
                                        colors.accent
                                    } else {
                                        colors.line
                                    }))
                                    .bg(rgb(if selected {
                                        colors.accent
                                    } else {
                                        colors.panel
                                    }))
                                    .text_color(rgb(if selected {
                                        colors.on_accent
                                    } else {
                                        colors.text
                                    }))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(move |el| {
                                        el.bg(rgb(if selected {
                                            colors.accent_hover
                                        } else {
                                            colors.hover
                                        }))
                                    })
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_theme(theme, window, cx)
                                    }))
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .id("single-instance-row")
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .py(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(px(12.)).child("싱글 인스턴스"))
                            .child(
                                div()
                                    .mt(px(4.))
                                    .text_size(px(11.))
                                    .text_color(rgb(colors.muted))
                                    .child("새로 실행할 때 기존 창에서 파일을 재생합니다."),
                            ),
                    )
                    .child(
                        div()
                            .id("single-instance-toggle")
                            .w(px(36.))
                            .h(px(22.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(if self.preferences.single_instance {
                                colors.accent
                            } else {
                                colors.line
                            }))
                            .p(px(3.))
                            .flex()
                            .when(self.preferences.single_instance, |el| el.justify_end())
                            .cursor_pointer()
                            .child(div().size(px(16.)).rounded_full().bg(rgb(colors.thumb)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.preferences.single_instance =
                                    !this.preferences.single_instance;
                                let enabled = this.preferences.single_instance;
                                this.persist(|settings| settings.single_instance = enabled);
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.settings_error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(colors.warning))
                        .child(error),
                )
            });
        div()
            .id("settings-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(colors.overlay))
            .p(px(12.))
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings_open = false;
                cx.notify();
            }))
            .child(
                div()
                    .id("settings-panel")
                    .occlude()
                    .w_full()
                    .max_w(px(480.))
                    .h_full()
                    .max_h(px(320.))
                    .flex()
                    .flex_col()
                    .rounded(px(10.))
                    .bg(rgb(colors.panel))
                    .border_1()
                    .border_color(rgb(colors.line))
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .h(px(36.))
                            .flex_shrink_0()
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(icon("settings", 18., colors.accent))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("EdolWave · 설정"),
                                    ),
                            )
                            .child(
                                tool(colors, "close-settings", "close").on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.settings_open = false;
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(body),
            )
    }

    fn info_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.palette;
        let shortcuts = [
            (
                "파일 열기",
                if cfg!(target_os = "macos") {
                    "Cmd + O"
                } else {
                    "Ctrl + O"
                },
            ),
            ("재생 / 일시정지", "Space"),
            ("5초 탐색", "← / →"),
            ("처음으로", "Home"),
            ("볼륨", "↑ / ↓"),
            ("음소거 / 반복", "M / R"),
            ("안내 닫기", "Esc"),
        ];
        let heading = |title| {
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title)
        };
        div()
            .id("info-overlay")
            .absolute().top_0().left_0().size_full()
            .occlude()
            .bg(rgba(colors.overlay))
            .p(px(12.))
            .flex().items_center().justify_center()
            .on_click(cx.listener(|this, _, _, cx| {
                this.info_open = false;
                cx.notify();
            }))
            .child(
                div().id("info-panel").occlude()
                    .w_full().max_w(px(760.)).h_full().max_h(px(280.))
                    .flex().flex_col()
                    .rounded(px(10.)).bg(rgb(colors.panel))
                    .border_1().border_color(rgb(colors.line)).shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div().h(px(36.)).flex_shrink_0().px(px(12.))
                            .flex().items_center().justify_between()
                            .child(div().flex().items_center().gap(px(8.))
                                .child(icon("info", 18., colors.accent))
                                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("EdolWave · 정보"))
                                .child(div().text_size(px(10.)).text_color(rgb(colors.muted)).child(env!("CARGO_PKG_VERSION"))))
                            .child(tool(colors, "close-info", "close").on_click(cx.listener(|this, _, _, cx| {
                                this.info_open = false;
                                cx.notify();
                            })))
                    )
                    .child(
                        div().id("info-body").flex_1().min_h_0().overflow_y_scroll()
                            .px(px(14.)).pb(px(12.))
                            .child(
                                div().flex().gap(px(14.))
                                    .child(
                                        div().flex_1().min_w_0().flex().flex_col()
                                            .child(heading("단축키").mb(px(6.)))
                                            .children(shortcuts.into_iter().map(|(label, key)| {
                                                div().h(px(20.)).flex_shrink_0().flex().items_center().justify_between().gap(px(6.))
                                                    .text_size(px(11.))
                                                    .child(label)
                                                    .child(div().flex_shrink_0().text_color(rgb(colors.muted)).child(key))
                                            }))
                                    )
                                    .child(div().w(px(1.)).flex_shrink_0().bg(rgb(colors.line)))
                                    .child(
                                        div().flex_1().min_w_0().flex().flex_col().gap(px(8.))
                                            .text_size(px(11.)).line_height(px(16.)).text_color(rgb(colors.muted))
                                            .child(heading("재생 안내").text_color(rgb(colors.text)))
                                            .child("파형을 클릭하거나 드래그해 탐색합니다.")
                                            .child("볼륨·배속 위에서 휠로 조절합니다.")
                                            .child("여러 파일은 이전·다음 버튼으로 이동합니다.")
                                            .child("WAV·MP3·FLAC·OGG/Vorbis·AAC·M4A·AIFF 등을 지원합니다.")
                                    )
                            )
                    )
            )
    }

    fn controls(&self, narrow: bool, cx: &mut Context<Self>) -> Div {
        let colors = self.palette;
        let transport = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(8.))
            .child(
                transport(colors, "previous", "previous", false)
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(-1, cx))),
            )
            .child(
                transport(
                    colors,
                    "play",
                    if self.state.playing { "pause" } else { "play" },
                    true,
                )
                .on_click(cx.listener(|this, _, w, cx| this.play(w, cx))),
            )
            .child(
                transport(colors, "next", "next", false)
                    .on_click(cx.listener(|this, _, _, cx| this.navigate(1, cx))),
            )
            .child(
                transport(colors, "stop", "stop", false).on_click(cx.listener(|this, _, _, cx| {
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
                    colors,
                    "mute",
                    if self.displayed_volume() == 0. {
                        "mute"
                    } else {
                        "volume"
                    },
                )
                .tooltip({
                    let percent = (self.displayed_volume() * 100.).round() as u32;
                    move |_, cx| {
                        cx.new(|_| TextTooltip(format!("볼륨 {percent}% · 음소거 전환"), colors))
                            .into()
                    }
                })
                .on_click(cx.listener(|this, _, _, cx| this.mute(cx))),
            )
            .child(self.volume(narrow, cx))
            .when(!narrow, |el| {
                el.child(
                    div()
                        .w(px(28.))
                        .text_size(px(11.))
                        .text_color(rgb(colors.muted))
                        .child(format!(
                            "{}%",
                            (self.displayed_volume() * 100.).round() as u32
                        )),
                )
            });
        let speed = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(if narrow { 6. } else { 10. }))
            .when(narrow, |el| el.child(icon("speed", 20., colors.muted)))
            .when(!narrow, |el| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(colors.muted))
                        .child("재생 속도"),
                )
            })
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
            .when(narrow && self.preferences.repeat, |el| {
                el.bg(rgb(colors.active))
            })
            .hover(move |s| s.bg(rgb(colors.panel)))
            .child(icon(
                "repeat",
                20.,
                if self.preferences.repeat {
                    colors.accent
                } else {
                    colors.muted
                },
            ))
            .when(!narrow, |el| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(colors.muted))
                        .child("반복 재생"),
                )
                .child(
                    div()
                        .w(px(32.))
                        .h(px(19.))
                        .rounded_full()
                        .bg(rgb(if self.preferences.repeat {
                            colors.accent
                        } else {
                            colors.line
                        }))
                        .p(px(3.))
                        .flex()
                        .when(self.preferences.repeat, |d| d.justify_end())
                        .child(div().size(px(13.)).rounded_full().bg(rgb(colors.thumb))),
                )
            })
            .tooltip({
                let label = if self.preferences.repeat {
                    "반복 재생 켜짐"
                } else {
                    "반복 재생 꺼짐"
                };
                move |_, cx| cx.new(|_| TextTooltip(label.into(), colors)).into()
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_repeat(cx)));
        div()
            .h(px(48.))
            .flex_shrink_0()
            .px(px(16.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(4.))
            .child(transport)
            .child(volume)
            .child(speed)
            .child(repeat)
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        if self.info_open || self.settings_open {
            if key == "escape" {
                self.info_open = false;
                self.settings_open = false;
                cx.notify();
            }
            return;
        }
        let command = if cfg!(target_os = "macos") {
            e.keystroke.modifiers.platform
        } else {
            e.keystroke.modifiers.control
        };
        if command && key == "o" {
            self.choose(window, cx);
            return;
        }
        if command && key == "q" {
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
            "r" => self.toggle_repeat(cx),
            "m" => self.mute(cx),
            "escape" => {
                self.info_open = false;
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
        let colors = self.palette;
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
            .min_h(px(32.))
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
                            colors,
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
                        .text_color(rgb(colors.muted))
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

    fn volume(&self, narrow: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.palette;
        let cell = self.volume_bounds.clone();
        let v = self.displayed_volume();
        div()
            .id("volume-bar")
            .relative()
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
                            rgb(colors.line),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        window.paint_quad(quad(
                            Bounds::new(point(b.origin.x, y), size(b.size.width * v, px(4.))),
                            px(2.),
                            rgb(colors.accent),
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
                            rgb(colors.text),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                    },
                )
                .size_full(),
            )
            .when(
                narrow && (self.volume_feedback_until.is_some() || self.volume_drag),
                |el| {
                    el.child(
                        div()
                            .id("volume-feedback")
                            .occlude()
                            .absolute()
                            .bottom(px(32.))
                            .left(px(16.))
                            .h(px(24.))
                            .min_w(px(48.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .bg(rgb(colors.panel))
                            .border_1()
                            .border_color(rgb(colors.line))
                            .shadow_lg()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(colors.text))
                            .cursor_default()
                            .child(format!("{}%", (v * 100.).round() as u32))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                    )
                },
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
        let colors = self.palette;
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
                    "{} Hz  ·  {}",
                    w.sample_rate,
                    if w.channels == 1 {
                        "Mono".into()
                    } else {
                        format!("{} ch", w.channels)
                    }
                )
            })
            .unwrap_or_else(|| "NATIVE AUDIO PLAYER".into());
        let narrow = window.viewport_size().width < px(640.);
        div()
            .id("player")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(colors.bg))
            .text_color(rgb(colors.text))
            .font_family(".SystemUIFont")
            .text_size(px(14.))
            .on_key_down(cx.listener(Self::key))
            .on_drop(cx.listener(|this, files: &ExternalPaths, _, cx| {
                this.open(files.paths().to_vec(), cx)
            }))
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if !e.pressed_button.is_some_and(|b| b == MouseButton::Left) {
                    if this.scrubbing.is_some() {
                        this.finish_seek(cx);
                    }
                    this.finish_volume_drag(cx);
                    return;
                }
                if this.scrubbing.is_some() {
                    this.scrubbing = Some(this.fraction(e.position.x));
                    cx.notify();
                }
                if this.volume_drag {
                    this.set_volume_at(e.position.x, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_seek(cx);
                    this.finish_volume_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_seek(cx);
                    this.finish_volume_drag(cx);
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .h(px(44.))
                    .px(px(16.))
                    .gap(px(4.))
                    .child(
                        div()
                            .id("title-drag")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .gap(px(14.))
                            .cursor_default()
                            .window_control_area(WindowControlArea::Drag)
                            .child(icon("wave", 26., colors.logo))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_ellipsis()
                                    .child(name),
                            ),
                    )
                    .child(
                        tool(colors, "open", "folder")
                            .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx))),
                    )
                    .child(
                        tool(colors, "info", "info").on_click(cx.listener(|this, _, _, cx| {
                            this.info_open = !this.info_open;
                            this.settings_open = false;
                            this.speed_menu_open = false;
                            cx.notify();
                        })),
                    )
                    .child(
                        tool(colors, "settings", "settings")
                            .relative()
                            .when(
                                matches!(self.update_check, CheckState::Available(_)),
                                |el| {
                                    el.child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .size(px(8.))
                                            .rounded_full()
                                            .bg(rgb(colors.accent))
                                            .border_1()
                                            .border_color(rgb(colors.bg)),
                                    )
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings_open = !this.settings_open;
                                this.info_open = false;
                                this.speed_menu_open = false;
                                if this.settings_open
                                    && matches!(
                                        this.update_check,
                                        CheckState::Current | CheckState::Failed(_)
                                    )
                                {
                                    this.check_updates(cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(1.)).h(px(20.)).bg(rgb(colors.line)).mx_1())
                    .child(
                        tool(colors, "minimize", "minimize")
                            .on_click(|_, w, _| w.minimize_window()),
                    )
                    .child(tool(colors, "maximize", "maximize").on_click(|_, w, _| w.zoom_window()))
                    .child(tool(colors, "close", "close").on_click(|_, _, cx| cx.quit())),
            )
            .child(
                div()
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .flex()
                    .flex_col()
                    .child(self.waveform(cx))
                    .child(
                        div()
                            .h(px(20.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex()
                                    .flex_shrink_0()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_size(px(12.))
                                    .line_height(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(div().text_color(rgb(colors.accent)).child(time(pos)))
                                    .child(div().text_color(rgb(colors.subtle)).child("/"))
                                    .child(
                                        div()
                                            .text_color(rgb(colors.muted))
                                            .child(time(self.state.duration)),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_size(px(10.))
                                    .text_color(rgb(colors.subtle))
                                    .child(metadata),
                            ),
                    ),
            )
            .child(self.controls(narrow, cx))
            .when_some(self.state.error.clone(), |el, error| {
                el.child(
                    div()
                        .h(px(24.))
                        .flex_shrink_0()
                        .px(px(28.))
                        .text_size(px(11.))
                        .text_color(rgb(colors.error))
                        .text_ellipsis()
                        .child(error),
                )
            })
            .when(self.speed_menu_open, |el| {
                el.child(self.speed_menu(window, cx))
            })
            .when(self.info_open, |el| el.child(self.info_overlay(cx)))
            .when(self.settings_open, |el| el.child(self.settings_overlay(cx)))
    }
}

fn icon(name: &'static str, size: f32, color: u32) -> Svg {
    svg().path(name).size(px(size)).text_color(rgb(color))
}
fn tool(colors: Palette, id: &'static str, name: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgb(if name == "close" {
                colors.danger_hover
            } else {
                colors.tool_hover
            }))
        })
        .child(icon(name, 19., colors.muted))
}
struct TextTooltip(String, Palette);
impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = self.1;
        div()
            .max_w(px(300.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(rgb(colors.panel))
            .border_1()
            .border_color(rgb(colors.line))
            .shadow_lg()
            .font_family(".SystemUIFont")
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(rgb(colors.text))
            .child(self.0.clone())
    }
}
fn settings_button(
    colors: Palette,
    id: &'static str,
    label: String,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(10.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(rgb(colors.accent))
        .text_color(rgb(colors.on_accent))
        .text_size(px(11.))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(move |el| el.bg(rgb(colors.accent_hover)))
        })
        .when(!enabled, |el| el.opacity(0.5))
        .child(label)
}
fn transport(
    colors: Palette,
    id: &'static str,
    name: &'static str,
    primary: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(32.))
        .h(px(24.))
        .flex_shrink_0()
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(if primary {
            colors.accent_border
        } else {
            colors.line
        }))
        .bg(rgb(if primary { colors.accent } else { colors.panel }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgb(if primary {
                colors.accent_hover
            } else {
                colors.hover
            }))
        })
        .child(icon(
            name,
            18.,
            if primary {
                colors.on_accent
            } else {
                colors.text
            },
        ))
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
