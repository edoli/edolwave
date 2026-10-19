#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod audio;
mod instance;
mod ui;
mod waveform;
mod waveform_view;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use std::{path::PathBuf, time::Instant};

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let Some((_instance, requests)) = instance::Instance::acquire(&files)? else {
        return Ok(());
    };
    Application::new()
        .with_assets(assets::Icons)
        .run(move |cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(640.), px(240.)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(480.), px(240.))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("EdolWave".into()),
                            appears_transparent: true,
                            ..Default::default()
                        }),
                        app_id: Some("dev.edoli.edolwave".into()),
                        ..Default::default()
                    },
                    move |window, cx| cx.new(|cx| ui::PlayerView::new(files, started, window, cx)),
                )
                .expect("플레이어 창을 열 수 없습니다");
            cx.spawn(async move |cx| {
                while let Ok(files) = requests.recv().await {
                    if window
                        .update(cx, |view, window, cx| {
                            view.open_from_instance(files, window, cx)
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .detach();
            cx.on_window_closed(|cx| cx.quit()).detach();
            cx.activate(true);
        });
    Ok(())
}
