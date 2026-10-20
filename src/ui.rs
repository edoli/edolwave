use crate::audio::{Audio, Command, Snapshot};
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
    info_open: bool,
    settings_open: bool,
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
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let audio = Audio::new();
        let state = audio.snapshot();
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
            info_open: false,
            settings_open: false,
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
                this.info_open = false;
                this.settings_open = false;
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

    fn settings_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, color, enabled, hint) = match &self.update_check {
            CheckState::Checking => (
                "확인 중…".to_owned(),
                MUTED,
                false,
                "최신 버전을 확인하고 있습니다.".to_owned(),
            ),
            CheckState::Current => (
                "최신 버전".to_owned(),
                0x65d6ac,
                false,
                "현재 최신 버전을 사용하고 있습니다.".to_owned(),
            ),
            CheckState::Failed(error) => ("확인 실패".to_owned(), 0xffb580, false, error.clone()),
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
                (label, BLUE, enabled, hint)
            }
        };
        let ready = matches!(self.update_install, InstallState::Ready(_));
        let action = if matches!(self.update_check, CheckState::Available(_)) {
            settings_button("install-update", label, enabled)
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
                            .tooltip(move |_, cx| cx.new(|_| SettingsTooltip(hint.clone())).into())
                            .child(action),
                    ),
            );
        div()
            .id("settings-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(rgba(0x080d16df))
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
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(LINE))
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
                                    .child(icon("settings", 18., BLUE))
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("EdolWave · 설정"),
                                    ),
                            )
                            .child(tool("close-settings", "close").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.settings_open = false;
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(body),
            )
    }

    fn info_overlay(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .bg(rgba(0x080d16df))
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
                    .rounded(px(10.)).bg(rgb(PANEL))
                    .border_1().border_color(rgb(LINE)).shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div().h(px(36.)).flex_shrink_0().px(px(12.))
                            .flex().items_center().justify_between()
                            .child(div().flex().items_center().gap(px(8.))
                                .child(icon("info", 18., BLUE))
                                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("EdolWave · 정보"))
                                .child(div().text_size(px(10.)).text_color(rgb(MUTED)).child(env!("CARGO_PKG_VERSION"))))
                            .child(tool("close-info", "close").on_click(cx.listener(|this, _, _, cx| {
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
                                                    .child(div().flex_shrink_0().text_color(rgb(MUTED)).child(key))
                                            }))
                                    )
                                    .child(div().w(px(1.)).flex_shrink_0().bg(rgb(LINE)))
                                    .child(
                                        div().flex_1().min_w_0().flex().flex_col().gap(px(8.))
                                            .text_size(px(11.)).line_height(px(16.)).text_color(rgb(MUTED))
                                            .child(heading("재생 안내").text_color(rgb(TEXT)))
                                            .child("파형을 클릭하거나 드래그해 탐색합니다.")
                                            .child("볼륨·배속 위에서 휠로 조절합니다. 배속은 0.5~2배이며 음높이도 바뀝니다.")
                                            .child("여러 파일은 이전·다음 버튼으로 이동합니다.")
                                            .child("WAV·MP3·FLAC·OGG/Vorbis·AAC·M4A·AIFF 등을 지원합니다.")
                                    )
                            )
                    )
            )
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
            "r" => self.audio.send(Command::Repeat),
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
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
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
                    this.volume_drag = false;
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
                    this.volume_drag = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_seek(cx);
                    this.volume_drag = false;
                }),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .h(px(44.))
                    .px(px(16.))
                    .gap(px(14.))
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
                            .child(icon("wave", 26., 0x49caff))
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
                        tool("open", "folder")
                            .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx))),
                    )
                    .child(tool("info", "info").on_click(cx.listener(|this, _, _, cx| {
                        this.info_open = !this.info_open;
                        this.settings_open = false;
                        this.speed_menu_open = false;
                        cx.notify();
                    })))
                    .child(
                        tool("settings", "settings")
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
                                            .bg(rgb(BLUE))
                                            .border_1()
                                            .border_color(rgb(BG)),
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
                    .child(div().w(px(1.)).h(px(20.)).bg(rgb(LINE)).mx_1())
                    .child(tool("minimize", "minimize").on_click(|_, w, _| w.minimize_window()))
                    .child(tool("maximize", "maximize").on_click(|_, w, _| w.zoom_window()))
                    .child(tool("close", "close").on_click(|_, _, cx| cx.quit())),
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
                                    .child(div().text_color(rgb(BLUE)).child(time(pos)))
                                    .child(div().text_color(rgb(0x65758c)).child("/"))
                                    .child(
                                        div()
                                            .text_color(rgb(MUTED))
                                            .child(time(self.state.duration)),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_size(px(10.))
                                    .text_color(rgb(0x65758c))
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
                        .text_color(rgb(0xffa9a9))
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
fn tool(id: &'static str, name: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(if name == "close" { 0x663142 } else { 0x263345 })))
        .child(icon(name, 19., MUTED))
}
struct SettingsTooltip(String);
impl Render for SettingsTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(300.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgb(LINE))
            .shadow_lg()
            .font_family(".SystemUIFont")
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(rgb(TEXT))
            .child(self.0.clone())
    }
}
fn settings_button(id: &'static str, label: String, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(10.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(rgb(BLUE))
        .text_color(rgb(TEXT))
        .text_size(px(11.))
        .when(enabled, |el| {
            el.cursor_pointer().hover(|el| el.bg(rgb(0x55a0ff)))
        })
        .when(!enabled, |el| el.opacity(0.5))
        .child(label)
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
