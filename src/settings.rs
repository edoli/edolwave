//! Small, per-user preferences shared by all player windows.
use crate::theme::Theme;
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub single_instance: bool,
    pub volume: f32,
    pub speed: f32,
    pub repeat: bool,
    pub theme: Theme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            single_instance: true,
            volume: 0.8,
            speed: 1.,
            repeat: false,
            theme: Theme::default(),
        }
    }
}

impl Settings {
    fn validate(mut self) -> Self {
        let defaults = Self::default();
        if !self.volume.is_finite() || !(0. ..=1.).contains(&self.volume) {
            self.volume = defaults.volume;
        }
        if !self.speed.is_finite() || !(0.5..=2.).contains(&self.speed) {
            self.speed = defaults.speed;
        }
        self
    }
}

pub struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn new() -> Result<Self> {
        let directory = match std::env::var_os("EDOLWAVE_CONFIG_DIR") {
            Some(directory) => PathBuf::from(directory),
            None => dirs::config_dir()
                .context("사용자 설정 폴더를 찾을 수 없습니다")?
                .join("EdolWave"),
        };
        Ok(Self { directory })
    }

    pub fn load(&self) -> Settings {
        let path = self.directory.join("settings.json");
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Settings>(&bytes) {
                Ok(settings) => settings.validate(),
                Err(error) => {
                    eprintln!("설정을 읽을 수 없습니다 ({}): {error}", path.display());
                    Settings::default()
                }
            },
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("설정을 읽을 수 없습니다 ({}): {error}", path.display());
                }
                Settings::default()
            }
        }
    }

    pub fn update(&self, change: impl FnOnce(&mut Settings)) -> Result<()> {
        fs::create_dir_all(&self.directory)?;
        // Serialize read/modify/write across independent processes, preserving
        // preferences changed by another window. Readers see only complete JSON.
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("settings.lock"))?;
        lock.lock()?;
        let mut settings = self.load();
        change(&mut settings);
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        serde_json::to_writer_pretty(&mut file, &settings.validate())?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(self.directory.join("settings.json"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_partial_invalid_and_corrupt_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store {
            directory: directory.path().into(),
        };
        assert_eq!(store.load(), Settings::default());
        let path = directory.path().join("settings.json");
        fs::write(&path, br#"{"repeat":true}"#).unwrap();
        assert_eq!(
            store.load(),
            Settings {
                repeat: true,
                ..Settings::default()
            }
        );
        fs::write(
            &path,
            br#"{"volume":-1,"speed":99,"single_instance":false}"#,
        )
        .unwrap();
        assert_eq!(
            store.load(),
            Settings {
                single_instance: false,
                ..Settings::default()
            }
        );
        fs::write(&path, b"broken JSON").unwrap();
        assert_eq!(store.load(), Settings::default());
    }

    #[test]
    fn concurrent_updates_merge_and_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            for index in 0..5 {
                let directory = directory.path();
                scope.spawn(move || {
                    let store = Store {
                        directory: directory.into(),
                    };
                    store
                        .update(|settings| match index {
                            0 => settings.single_instance = false,
                            1 => settings.volume = 0.35,
                            2 => settings.speed = 1.75,
                            3 => settings.repeat = true,
                            _ => settings.theme = Theme::Dark,
                        })
                        .unwrap();
                });
            }
        });
        let store = Store {
            directory: directory.path().into(),
        };
        assert_eq!(
            store.load(),
            Settings {
                single_instance: false,
                volume: 0.35,
                speed: 1.75,
                repeat: true,
                theme: Theme::Dark,
            }
        );
    }

    #[test]
    fn theme_choices_survive_reopening_and_unknown_themes_keep_other_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store {
            directory: directory.path().into(),
        };
        for theme in [Theme::Light, Theme::Dark, Theme::System] {
            store.update(|settings| settings.theme = theme).unwrap();
            let reopened = Store {
                directory: directory.path().into(),
            };
            assert_eq!(reopened.load().theme, theme);
        }
        fs::write(
            directory.path().join("settings.json"),
            br#"{"theme":"future_theme","volume":0.35}"#,
        )
        .unwrap();
        let settings = store.load();
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(settings.volume, 0.35);
    }
}
