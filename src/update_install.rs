//! Prepare a separate updater before allowing the UI to close.
use anyhow::{Context as _, Result, bail, ensure};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub const RESULT_FILE: &str = "update-result.txt";

pub struct PreparedUpdate {
    directory: Option<tempfile::TempDir>,
    child: Child,
}

impl PreparedUpdate {
    pub fn commit(mut self) -> Result<()> {
        ensure!(
            self.child.try_wait()?.is_none(),
            "업데이트 도우미가 종료되었습니다. 다시 시도해 주세요."
        );
        let directory = self.directory.as_ref().unwrap().path();
        fs::write(directory.join("go.pending"), b"install")?;
        fs::rename(directory.join("go.pending"), directory.join("go"))?;
        // The helper now owns cleanup, including after a failed installation.
        let _ = self.directory.take().unwrap().keep();
        Ok(())
    }
}

impl Drop for PreparedUpdate {
    fn drop(&mut self) {
        if self.directory.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[derive(Serialize)]
struct UpdateContext {
    executable: PathBuf,
    package: PathBuf,
    version: String,
    sha256: String,
    parent: u32,
    arguments: Vec<PathBuf>,
    config: PathBuf,
}

pub fn take_failure(config: &Path) -> Option<String> {
    let path = config.join(RESULT_FILE);
    let message = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    (!message.trim().is_empty()).then(|| message.chars().take(4096).collect())
}

pub fn prepare(
    directory: tempfile::TempDir,
    package: PathBuf,
    version: String,
    digest: [u8; 32],
) -> Result<PreparedUpdate> {
    let executable = if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .map_or_else(std::env::current_exe, Ok)?
    } else {
        std::env::current_exe()?
    };
    let store = crate::settings::Store::new()?;
    let context = UpdateContext {
        executable: std::path::absolute(executable)?,
        package: std::path::absolute(package)?,
        version,
        sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
        parent: std::process::id(),
        arguments: std::env::args_os().skip(1).map(PathBuf::from).collect(),
        config: std::path::absolute(store.directory())?,
    };
    prepare_context(directory, context)
}

fn prepare_context(directory: tempfile::TempDir, context: UpdateContext) -> Result<PreparedUpdate> {
    fs::create_dir_all(&context.config)?;
    let root = std::path::absolute(directory.path())?;
    fs::write(root.join("context.json"), serde_json::to_vec(&context)?)?;

    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        fs::write(
            root.join("update.ps1"),
            include_str!("../installer/update.ps1"),
        )?;
        let shell = PathBuf::from(
            std::env::var_os("SystemRoot").context("Windows 경로를 찾을 수 없습니다")?,
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(shell);
        command.args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command",
            "& ([ScriptBlock]::Create([IO.File]::ReadAllText((Join-Path $env:EDOLWAVE_UPDATE_DIR 'update.ps1'))))"])
            .creation_flags(0x08000000);
        command
    };
    #[cfg(unix)]
    let mut command = {
        fs::write(
            root.join("update.sh"),
            include_str!("../installer/update.sh"),
        )?;
        let mut restart = format!("#!/bin/sh\nexec {}", sh_quote(&context.executable)?);
        for argument in &context.arguments {
            restart.push(' ');
            restart.push_str(&sh_quote(argument)?);
        }
        restart.push('\n');
        fs::write(root.join("restart.sh"), restart)?;
        let mut command = Command::new("sh");
        command
            .arg(root.join("update.sh"))
            .env("EDOLWAVE_UPDATE_EXE", &context.executable)
            .env("EDOLWAVE_UPDATE_PACKAGE", &context.package)
            .env("EDOLWAVE_UPDATE_VERSION", &context.version)
            .env("EDOLWAVE_UPDATE_HASH", &context.sha256)
            .env("EDOLWAVE_UPDATE_PARENT", context.parent.to_string());
        command
    };
    let child = command
        .env("EDOLWAVE_UPDATE_DIR", &root)
        .env("EDOLWAVE_CONFIG_DIR", &context.config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("업데이트 도우미를 시작하지 못했습니다")?;
    let mut prepared = PreparedUpdate {
        directory: Some(directory),
        child,
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if prepared.child.try_wait()?.is_some() {
            bail!(
                "{}",
                take_failure(&context.config)
                    .unwrap_or_else(|| "업데이트 도우미가 준비 중 종료되었습니다.".into())
            );
        }
        if root.join("ready").is_file() {
            return Ok(prepared);
        }
        ensure!(
            Instant::now() < deadline,
            "업데이트 도우미 응답 시간이 초과되었습니다."
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture(valid: bool) -> (tempfile::TempDir, UpdateContext) {
        let directory = tempfile::Builder::new()
            .prefix("edolwave-update-")
            .tempdir()
            .unwrap();
        let package = directory.path().join("edolwave-windows.zip");
        fs::write(&package, b"verified fixture").unwrap();
        let config = directory.path().join("config");
        let digest = Sha256::digest(if valid {
            b"verified fixture".as_slice()
        } else {
            b"tampered".as_slice()
        });
        let context = UpdateContext {
            executable: std::env::current_exe().unwrap(),
            package,
            version: "9.9.9".into(),
            sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
            parent: std::process::id(),
            arguments: vec![],
            config,
        };
        (directory, context)
    }

    #[test]
    fn abandoning_a_ready_update_never_installs_and_cleans_up() {
        let (directory, context) = fixture(true);
        let root = directory.path().to_path_buf();
        let prepared = prepare_context(directory, context).unwrap();
        assert!(root.join("ready").exists());
        assert!(!root.join("go").exists());
        drop(prepared);
        assert!(!root.exists());
    }

    #[test]
    fn a_dead_helper_cannot_commit_and_close_the_app() {
        let (directory, context) = fixture(true);
        let root = directory.path().to_path_buf();
        let mut prepared = prepare_context(directory, context).unwrap();
        prepared.child.kill().unwrap();
        prepared.child.wait().unwrap();
        assert!(prepared.commit().is_err());
        assert!(!root.exists());
    }

    #[test]
    fn tampering_is_rejected_before_handoff() {
        let (directory, context) = fixture(false);
        assert!(prepare_context(directory, context).is_err());
    }
}

#[cfg(unix)]
fn sh_quote(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("업데이트 경로가 유효한 UTF-8 문자열이 아닙니다")?;
    Ok(format!("'{}'", text.replace('\'', "'\\''")))
}
