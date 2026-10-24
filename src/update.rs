use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const API: &str = "https://api.github.com/repos/edoli/edolwave";
const MAX_PACKAGE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    asset: Asset,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

pub enum CheckState {
    Checking,
    Current,
    Available(Release),
    Failed(String),
}

pub enum InstallState {
    Idle,
    Downloading { received: u64, total: u64 },
    Preparing,
    Failed(String),
}

pub enum Event {
    Checked(Result<Option<Release>, String>),
    Progress { received: u64, total: u64 },
    Preparing,
    Prepared(Result<crate::update_install::PreparedUpdate, String>),
}

pub struct Package {
    directory: tempfile::TempDir,
    pub path: PathBuf,
    version: String,
    digest: [u8; 32],
}

/// Networking and hashing run on worker threads, never the GPUI thread.
#[derive(Clone)]
pub struct Service {
    api: String,
    token: Option<String>,
}

impl Service {
    pub fn new() -> Self {
        Self {
            api: {
                // A local release fixture is available only in debug builds.
                #[cfg(debug_assertions)]
                let api = std::env::var("EDOLWAVE_UPDATE_TEST_API")
                    .ok()
                    .filter(|url| url.starts_with("http://127.0.0.1:"));
                #[cfg(not(debug_assertions))]
                let api: Option<String> = None;
                api.unwrap_or_else(|| API.to_owned())
            },
            token: std::env::var("EDOLWAVE_GITHUB_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }
    }

    fn agent(&self, timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("EdolWave/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    }

    fn get(
        &self,
        url: &str,
        binary: bool,
        timeout: Duration,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let agent = self.agent(timeout);
        let mut request = agent.get(url).header(
            "Accept",
            if binary {
                "application/octet-stream"
            } else {
                "application/vnd.github+json"
            },
        );
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        match request.call() {
            Ok(response) => Ok(response),
            Err(ureq::Error::StatusCode(404)) => {
                bail!("릴리스를 찾을 수 없습니다. 비공개 저장소는 GitHub 접근 권한이 필요합니다.")
            }
            Err(ureq::Error::StatusCode(401 | 403)) => bail!(
                "GitHub 접근 권한 또는 요청 한도를 확인해 주세요. 잠시 후 다시 시도할 수 있습니다."
            ),
            // Do not expose authorization headers or response contents in UI errors.
            Err(_) => {
                bail!("서버에 연결하지 못했습니다. 인터넷 연결을 확인하고 다시 시도해 주세요.")
            }
        }
    }

    pub fn check(&self) -> Result<Option<Release>> {
        let mut response = self.get(
            &format!("{}/releases/latest", self.api),
            false,
            Duration::from_secs(20),
        )?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .context("업데이트 응답을 읽지 못했습니다.")?;
        select_release(&bytes, env!("CARGO_PKG_VERSION"), package_name())
    }

    pub fn check_in_background(&self, events: async_channel::Sender<Event>) {
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service.check().map_err(|error| error.to_string());
            let _ = events.send_blocking(Event::Checked(result));
        });
    }

    pub fn update_in_background(&self, release: Release, events: async_channel::Sender<Event>) {
        let service = self.clone();
        std::thread::spawn(move || {
            let result = service
                .download(&release, |received, total| {
                    let _ = events.try_send(Event::Progress { received, total });
                })
                .and_then(|package| {
                    events.send_blocking(Event::Preparing)?;
                    crate::update_install::prepare(
                        package.directory,
                        package.path,
                        package.version,
                        package.digest,
                    )
                })
                .map_err(|error| format!("{error:#}"));
            let _ = events.send_blocking(Event::Prepared(result));
        });
    }

    fn download(&self, release: &Release, progress: impl FnMut(u64, u64)) -> Result<Package> {
        let cache = dirs::cache_dir()
            .context("다운로드 폴더를 찾을 수 없습니다.")?
            .join("edolwave")
            .join("updates");
        self.download_into(release, &cache, progress)
    }

    fn download_into(
        &self,
        release: &Release,
        cache: &Path,
        progress: impl FnMut(u64, u64),
    ) -> Result<Package> {
        let digest = parse_digest(release.asset.digest.as_deref())?;
        fs::create_dir_all(cache).context("다운로드 폴더를 만들지 못했습니다.")?;
        let directory = tempfile::Builder::new()
            .prefix("edolwave-update-")
            .tempdir_in(cache)
            .context("다운로드 폴더를 만들지 못했습니다.")?;
        let path = directory.path().join(&release.asset.name);
        let mut response = self.get(
            &format!("{}/releases/assets/{}", self.api, release.asset.id),
            true,
            Duration::from_secs(300),
        )?;
        let mut output = File::create(&path).context("설치 파일을 저장하지 못했습니다.")?;
        transfer_verified(
            response.body_mut().as_reader(),
            &mut output,
            release.asset.size,
            digest,
            progress,
        )?;
        output
            .sync_all()
            .context("설치 파일을 저장하지 못했습니다.")?;
        Ok(Package {
            directory,
            path,
            version: release.version.to_string(),
            digest,
        })
    }
}

fn select_release(bytes: &[u8], current: &str, package: &str) -> Result<Option<Release>> {
    let release: ApiRelease =
        serde_json::from_slice(bytes).context("업데이트 응답 형식이 올바르지 않습니다.")?;
    ensure!(
        !release.draft && !release.prerelease,
        "정식 릴리스 정보를 확인하지 못했습니다."
    );
    let version = Version::parse(release.tag_name.trim_start_matches('v'))
        .context("최신 버전 정보를 확인하지 못했습니다.")?;
    ensure!(
        version.pre.is_empty(),
        "정식 릴리스 정보를 확인하지 못했습니다."
    );
    if version <= Version::parse(current)? {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == package)
        .context("현재 플랫폼의 업데이트 파일이 아직 준비되지 않았습니다.")?;
    ensure!(
        asset.size > 0 && asset.size <= MAX_PACKAGE_SIZE,
        "설치 파일 크기가 올바르지 않습니다."
    );
    parse_digest(asset.digest.as_deref())?;
    Ok(Some(Release { version, asset }))
}

fn parse_digest(value: Option<&str>) -> Result<[u8; 32]> {
    let hex = value
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("업데이트 파일의 SHA-256 검증 정보가 없습니다.")?;
    ensure!(
        hex.len() == 64 && hex.is_ascii(),
        "SHA-256 검증 정보가 올바르지 않습니다."
    );
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .context("SHA-256 검증 정보가 올바르지 않습니다.")?;
    }
    Ok(digest)
}

fn transfer_verified(
    mut input: impl Read,
    output: &mut impl Write,
    total: u64,
    expected: [u8; 32],
    mut progress: impl FnMut(u64, u64),
) -> Result<()> {
    ensure!(
        total > 0 && total <= MAX_PACKAGE_SIZE,
        "설치 파일 크기가 올바르지 않습니다."
    );
    let mut hash = Sha256::new();
    let mut received = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input
            .read(&mut buffer)
            .context("다운로드가 중단되었습니다. 다시 시도해 주세요.")?;
        if count == 0 {
            break;
        }
        received += count as u64;
        ensure!(received <= total, "다운로드 파일 크기가 일치하지 않습니다.");
        output
            .write_all(&buffer[..count])
            .context("설치 파일을 저장하지 못했습니다.")?;
        hash.update(&buffer[..count]);
        progress(received, total);
    }
    ensure!(
        received == total,
        "다운로드가 완료되지 않았습니다. 다시 시도해 주세요."
    );
    ensure!(
        hash.finalize().as_slice() == expected,
        "파일 검증에 실패했습니다. 다시 다운로드해 주세요."
    );
    Ok(())
}

pub fn package_name() -> &'static str {
    #[cfg(windows)]
    {
        if std::env::current_exe()
            .ok()
            .is_some_and(|exe| crate::windows_install::install_scope(&exe).is_some())
        {
            "edolwave-installer-exe.exe"
        } else {
            "edolwave-windows.zip"
        }
    }
    #[cfg(target_os = "macos")]
    {
        if std::env::current_exe().ok().is_some_and(|exe| {
            exe.ancestors()
                .any(|path| path.extension().is_some_and(|ext| ext == "app"))
        }) {
            "edolwave-installer-dmg.dmg"
        } else {
            "edolwave-mac.zip"
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if std::env::var_os("APPIMAGE").is_some() {
            "edolwave-installer-appimage.AppImage"
        } else if std::env::current_exe()
            .ok()
            .is_some_and(|exe| exe == Path::new("/usr/bin/edolwave"))
        {
            "edolwave-installer-deb.deb"
        } else {
            "edolwave-linux.zip"
        }
    }
}

pub fn install_hint() -> &'static str {
    "업데이트를 누르면 다운로드·검증 후 자동 설치하고 다시 실행합니다. 필요한 경우 운영체제 권한 확인이 표시됩니다."
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str) -> serde_json::Value {
        json!({"tag_name": tag, "draft": false, "prerelease": false, "assets": [
            {"id": 123, "name": "edolwave-installer-exe.exe", "size": 3,
             "digest": format!("sha256:{:x}", Sha256::digest(b"abc"))}
        ]})
    }

    #[test]
    fn versions_use_semver_and_never_downgrade() {
        let choose = |tag| {
            select_release(
                &serde_json::to_vec(&release(tag)).unwrap(),
                "0.1.9",
                "edolwave-installer-exe.exe",
            )
        };
        assert_eq!(
            choose("v0.1.10").unwrap().unwrap().version,
            Version::new(0, 1, 10)
        );
        assert!(choose("v0.1.9").unwrap().is_none());
        assert!(choose("v0.1.8").unwrap().is_none());
        assert!(choose("v0.2.0-beta.1").is_err());
        assert!(choose("invalid").is_err());
    }

    #[test]
    fn incomplete_or_untrusted_release_is_not_reported_as_current() {
        for field in ["draft", "prerelease"] {
            let mut value = release("v0.2.0");
            value[field] = json!(true);
            assert!(
                select_release(
                    &serde_json::to_vec(&value).unwrap(),
                    "0.1.0",
                    "edolwave-installer-exe.exe"
                )
                .is_err()
            );
        }
        let mut value = release("v0.2.0");
        value["assets"][0]["digest"] = json!(null);
        assert!(
            select_release(
                &serde_json::to_vec(&value).unwrap(),
                "0.1.0",
                "edolwave-installer-exe.exe"
            )
            .is_err()
        );
        assert!(
            select_release(
                &serde_json::to_vec(&release("v0.2.0")).unwrap(),
                "0.1.0",
                "missing.dmg"
            )
            .is_err()
        );
        assert!(parse_digest(Some(&format!("sha256:{}", "é".repeat(32)))).is_err());
    }

    #[test]
    fn download_rejects_corruption_truncation_and_extra_bytes() {
        let expected: [u8; 32] = Sha256::digest(b"abc").into();
        let mut progress = Vec::new();
        let mut output = Vec::new();
        transfer_verified(&b"abc"[..], &mut output, 3, expected, |a, b| {
            progress.push((a, b))
        })
        .unwrap();
        assert_eq!(output, b"abc");
        assert_eq!(progress.last(), Some(&(3, 3)));
        for bytes in [&b"abd"[..], &b"ab"[..], &b"abcd"[..]] {
            assert!(transfer_verified(bytes, &mut Vec::new(), 3, expected, |_, _| {}).is_err());
        }
    }

    fn server(responses: Vec<(u16, Vec<u8>)>) -> (Service, std::thread::JoinHandle<Vec<String>>) {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let service = Service {
            api: format!("http://{}", listener.local_addr().unwrap()),
            token: Some("test-token".into()),
        };
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let mut connection = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    }
                };
                connection.set_nonblocking(false).unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    connection.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                requests.push(String::from_utf8(request).unwrap());
                write!(
                    connection,
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                connection.write_all(&body).unwrap();
            }
            requests
        });
        (service, handle)
    }

    #[test]
    fn http_checks_downloads_verifies_and_cleans_temporary_files() {
        let mut metadata = release("v9.9.9");
        metadata["assets"][0]["name"] = json!(package_name());
        let (service, server) = server(vec![
            (200, serde_json::to_vec(&metadata).unwrap()),
            (200, b"abc".to_vec()),
        ]);
        let release = service.check().unwrap().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let package = service
            .download_into(&release, cache.path(), |_, _| {})
            .unwrap();
        let path = package.path.clone();
        assert_eq!(fs::read(&path).unwrap(), b"abc");
        drop(package);
        assert!(!path.exists());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /releases/latest "));
        assert!(requests[1].starts_with("GET /releases/assets/123 "));
        assert!(
            requests[0]
                .to_lowercase()
                .contains("authorization: bearer test-token")
        );
        assert!(
            requests[1]
                .to_lowercase()
                .contains("accept: application/octet-stream")
        );
    }

    #[test]
    fn http_errors_and_bad_downloads_never_become_success() {
        for status in [401, 403, 404, 500] {
            let (service, server) = server(vec![(status, b"secret server detail".to_vec())]);
            let error = service.check().unwrap_err().to_string();
            assert!(!error.contains("secret"));
            assert!(!error.contains("test-token"));
            server.join().unwrap();
        }
        let (service, server) = server(vec![(200, b"abd".to_vec())]);
        let release = select_release(
            &serde_json::to_vec(&release("v9.9.9")).unwrap(),
            "0.1.0",
            "edolwave-installer-exe.exe",
        )
        .unwrap()
        .unwrap();
        let cache = tempfile::tempdir().unwrap();
        assert!(
            service
                .download_into(&release, cache.path(), |_, _| {})
                .is_err()
        );
        assert_eq!(fs::read_dir(cache.path()).unwrap().count(), 0);
        server.join().unwrap();
    }

    #[test]
    #[ignore = "downloads a GitHub release; private repos require EDOLWAVE_GITHUB_TOKEN"]
    fn github_release_can_be_checked_and_downloaded() {
        let service = Service::new();
        let mut response = service
            .get(
                &format!("{}/releases/latest", service.api),
                false,
                Duration::from_secs(20),
            )
            .unwrap();
        let bytes = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .unwrap();
        let release = select_release(&bytes, "0.0.0", package_name())
            .unwrap()
            .unwrap();
        let cache = tempfile::tempdir().unwrap();
        let package = service
            .download_into(&release, cache.path(), |_, _| {})
            .unwrap();
        assert_eq!(
            fs::metadata(&package.path).unwrap().len(),
            release.asset.size
        );
    }
}
