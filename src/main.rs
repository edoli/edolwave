#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assets;
mod audio;
mod instance;
mod settings;
mod theme;
mod ui;
mod update;
mod waveform;
mod waveform_view;
mod wsola;

use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};
use std::{path::PathBuf, time::Instant};

fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let settings_store = settings::Store::new()?;
    let settings = settings_store.load();
    let Some((_instance, requests)) =
        instance::Instance::acquire(&files, settings.single_instance)?
    else {
        return Ok(());
    };
    let application = Application::new().with_assets(assets::Icons);
    #[cfg(target_os = "macos")]
    {
        let requests = _instance.request_sender();
        application.on_open_urls(move |urls| {
            let files: Vec<PathBuf> = urls
                .iter()
                .filter_map(|value| url::Url::parse(value).ok()?.to_file_path().ok())
                .collect();
            if !files.is_empty() {
                if let Err(error) = requests.try_send(files) {
                    eprintln!("파일 열기 요청을 전달하지 못했습니다: {error}");
                }
            }
        });
        let requests = _instance.request_sender();
        application.on_reopen(move |_| {
            if let Err(error) = requests.try_send(Vec::new()) {
                eprintln!("창 활성화 요청을 전달하지 못했습니다: {error}");
            }
        });
    }
    application.run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(640.), px(240.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(480.), px(180.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("EdolWave".into()),
                        appears_transparent: !cfg!(target_os = "macos"),
                        ..Default::default()
                    }),
                    app_id: Some("dev.edoli.edolwave".into()),
                    ..Default::default()
                },
                move |window, cx| {
                    cx.new(|cx| {
                        ui::PlayerView::new(files, settings_store, settings, started, window, cx)
                    })
                },
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
