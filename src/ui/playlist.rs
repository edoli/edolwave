use super::{PlayerView, TextTooltip, icon, tool};
use gpui::{
    Context, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, ScrollStrategy,
    UniformListScrollHandle, WeakEntity, Window, WindowControlArea, div, prelude::*, px, rgb,
    uniform_list,
};

pub(super) struct PlaylistView {
    player: WeakEntity<PlayerView>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl PlaylistView {
    pub(super) fn new(
        player: Entity<PlayerView>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        // Playback notifies every frame; redraw this window only when its visible
        // state changes. Keep paths in the player rather than copying the queue.
        let mut previous = None;
        cx.observe(&player, move |this, player, cx| {
            let player = player.read(cx);
            let state = (
                player.playlist_revision,
                player.index,
                player.state.playing,
                player.state.loading,
                player.palette,
            );
            if previous != Some(state) {
                if previous.is_none_or(|old: (u64, usize, bool, bool, super::Palette)| {
                    (old.0, old.1) != (state.0, state.1)
                }) {
                    this.scroll
                        .scroll_to_item(player.index, ScrollStrategy::Center);
                }
                previous = Some(state);
                cx.notify();
            }
        })
        .detach();
        let scroll = UniformListScrollHandle::new();
        scroll.scroll_to_item(index, ScrollStrategy::Center);
        Self {
            player: player.downgrade(),
            focus,
            scroll,
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let command = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        if key == "escape" || (command && key == "l") {
            window.remove_window();
            return;
        }
        let _ = self.player.update(cx, |player, cx| match key {
            "o" if command => player.choose_files(true, cx),
            "up" => player.navigate(-1, cx),
            "down" => player.navigate(1, cx),
            "home" => player.play_index(0, cx),
            "end" => player.play_index(player.playlist.len().saturating_sub(1), cx),
            _ => player.key(event, window, cx),
        });
    }

    fn rows(
        &mut self,
        range: std::ops::Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Stateful<gpui::Div>> {
        let Some(player) = self.player.upgrade() else {
            return vec![];
        };
        let player = player.read(cx);
        let colors = player.palette;
        range
            .filter_map(|index| {
                let path = player.playlist.get(index)?;
                let name = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                let full_path = path.display().to_string();
                let current = index == player.index;
                Some(
                    div()
                        .id(("track", index))
                        .w_full()
                        .h(px(36.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .border_b_1()
                        .border_color(rgb(colors.line))
                        .when(current, |el| el.bg(rgb(colors.active)))
                        .hover(move |el| {
                            el.bg(rgb(if current { colors.active } else { colors.hover }))
                        })
                        .cursor_pointer()
                        .tooltip(move |_, cx| {
                            cx.new(|_| TextTooltip(full_path.clone(), colors)).into()
                        })
                        .child(
                            div()
                                .w(px(28.))
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(if current { colors.accent } else { colors.muted }))
                                .child(if current {
                                    "▶".to_owned()
                                } else {
                                    (index + 1).to_string()
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_ellipsis()
                                        .when(current, |el| el.font_weight(FontWeight::SEMIBOLD))
                                        .child(name),
                                ),
                        )
                        .when(current, |el| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(colors.accent))
                                    .child(if player.state.loading {
                                        "여는 중"
                                    } else if player.state.playing {
                                        "재생 중"
                                    } else {
                                        "선택됨"
                                    }),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let _ = this
                                .player
                                .update(cx, |player, cx| player.play_index(index, cx));
                        })),
                )
            })
            .collect()
    }
}

impl Render for PlaylistView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(player) = self.player.upgrade() else {
            return div().into_any_element();
        };
        let player = player.read(cx);
        let colors = player.palette;
        let count = player.playlist.len();
        div()
            .id("playlist-window")
            .on_key_down(cx.listener(Self::key))
            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                let _ = this
                    .player
                    .update(cx, |player, cx| player.append(paths.paths().to_vec(), cx));
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.bg))
            .text_color(rgb(colors.text))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .h(px(44.))
                    .px(px(12.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("playlist-title")
                            .h_full()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .window_control_area(WindowControlArea::Drag)
                            .child(icon("playlist", 20., colors.accent))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("재생 목록"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(colors.muted))
                                    .child(format!("{count}곡")),
                            ),
                    )
                    .child(
                        tool(colors, "open-playlist-files", "folder").on_click(cx.listener(
                            |this, _, _, cx| {
                                let _ = this
                                    .player
                                    .update(cx, |player, cx| player.choose_files(true, cx));
                            },
                        )),
                    )
                    .child(
                        tool(colors, "close-playlist", "close")
                            .on_click(|_, window, _| window.remove_window()),
                    ),
            )
            .child(
                div()
                    // Focusing a Div consumes mouse down's native default action.
                    // Keep the titlebar outside it so Windows can move the window.
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(rgb(colors.line))
                    .when(count == 0, |el| {
                        el.flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap(px(10.))
                            .child(icon("playlist", 32., colors.muted))
                            .child(div().text_size(px(14.)).child("재생 목록이 비어 있습니다"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(colors.muted))
                                    .child("파일을 끌어 놓거나 폴더 버튼으로 열어 주세요"),
                            )
                    })
                    .when(count > 0, |el| {
                        el.child(
                            uniform_list("playlist-tracks", count, cx.processor(Self::rows))
                                .size_full()
                                .track_scroll(self.scroll.clone()),
                        )
                    }),
            )
            .into_any_element()
    }
}
