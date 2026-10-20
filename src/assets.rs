use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub struct Icons;
impl AssetSource for Icons {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        let shape = match path {
            "play" => r#"<path d="m9 5 11 7-11 7Z" fill="white" stroke="none"/>"#,
            "pause" => {
                r#"<rect x="6" y="5" width="4" height="14" rx="1" fill="white" stroke="none"/><rect x="14" y="5" width="4" height="14" rx="1" fill="white" stroke="none"/>"#
            }
            "stop" => {
                r#"<rect x="6" y="6" width="12" height="12" rx="1" fill="white" stroke="none"/>"#
            }
            "previous" => {
                r#"<path d="M6 5v14"/><path d="m18 5-10 7 10 7Z" fill="white" stroke="none"/>"#
            }
            "next" => {
                r#"<path d="M18 5v14"/><path d="m6 5 10 7-10 7Z" fill="white" stroke="none"/>"#
            }
            "folder" => r#"<path d="M3 8V5h6l2 3h10v12H3Z"/><path d="M3 10h18"/>"#,
            "volume" => {
                r#"<path d="M3 9h4l5-4v14l-5-4H3Z"/><path d="M16 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14"/>"#
            }
            "mute" => r#"<path d="M3 9h4l5-4v14l-5-4H3Z"/><path d="m17 9 5 6m0-6-5 6"/>"#,
            "repeat" => {
                r#"<path d="m17 2 4 4-4 4M3 11V9a3 3 0 0 1 3-3h15M7 22l-4-4 4-4m14-1v2a3 3 0 0 1-3 3H3"/>"#
            }
            "speed" => {
                r#"<path d="M4.2 18a9 9 0 1 1 15.6 0M4.2 18h15.6M12 4v2M5.7 7l1.4 1.4M18.3 7l-1.4 1.4M12 13l4-4"/><circle cx="12" cy="13" r="1.2" fill="white" stroke="none"/>"#
            }
            "chevron" => r#"<path d="m6 9 6 6 6-6"/>"#,
            "close" => r#"<path d="m6 6 12 12M18 6 6 18"/>"#,
            "minimize" => r#"<path d="M5 12h14"/>"#,
            "maximize" => r#"<rect x="5" y="5" width="14" height="14" rx="1"/>"#,
            "info" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.1"/>"#,
            "settings" => {
                r#"<path d="m9.5 3-.5 2-1.6.9-2-.6-2.5 4.4 1.5 1.4v1.8l-1.5 1.4 2.5 4.4 2-.6 1.6.9.5 2h5l.5-2 1.6-.9 2 .6 2.5-4.4-1.5-1.4v-1.8l1.5-1.4-2.5-4.4-2 .6L15 5l-.5-2Z"/><circle cx="12" cy="12" r="3"/>"#
            }
            "wave" => r#"<path d="M2 10v4m4-8v12m4-16v20m4-17v14m4-11v8m4-6v4"/>"#,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Owned(format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{shape}</svg>"#).into_bytes())))
    }
    fn list(&self, _: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(vec![])
    }
}
