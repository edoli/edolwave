//! Hold a per-user file lock and forward subsequent launches over local sockets.
use anyhow::{Context as _, Result, bail};
use async_channel::{Receiver, Sender};
use interprocess::local_socket::{
    ConnectOptions, ListenerOptions, Name,
    tokio::{Listener, Stream, prelude::*},
};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    runtime::{Builder, Runtime},
    sync::oneshot,
    time::timeout,
};

const MAX_REQUEST: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Instance {
    // Drop the listener and join its thread before releasing the process lock.
    _lock: File,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
    #[cfg(target_os = "macos")]
    requests: Sender<Vec<PathBuf>>,
}

type Primary = (Instance, Receiver<Vec<PathBuf>>);

fn hash(value: impl Hash) -> u64 {
    let mut hash = DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

fn socket_name(directory: &Path) -> std::io::Result<Name<'static>> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::{GenericNamespaced, ToNsName};
        format!("dev.edoli.edolwave-{:016x}", hash(directory)).to_ns_name::<GenericNamespaced>()
    }
    #[cfg(unix)]
    {
        use interprocess::local_socket::{GenericFilePath, ToFsName};
        directory
            .join("instance.sock")
            .to_fs_name::<GenericFilePath>()
    }
}

impl Instance {
    #[cfg(target_os = "macos")]
    pub fn request_sender(&self) -> Sender<Vec<PathBuf>> {
        self.requests.clone()
    }

    /// None means the existing process accepted this launch; no UI should be created.
    pub fn acquire(files: &[PathBuf]) -> Result<Option<Primary>> {
        let mut directory = dirs::cache_dir()
            .context("사용자 캐시 폴더를 찾을 수 없습니다")?
            .join("EdolWave")
            .join("instance");
        // Isolate automated app tests from an installed player.
        if let Some(scope) = std::env::var_os("EDOLWAVE_INSTANCE_SCOPE") {
            directory.push(format!("test-{:016x}", hash(scope)));
        }
        Self::acquire_in(&directory, files)
    }

    fn acquire_in(directory: &Path, files: &[PathBuf]) -> Result<Option<Primary>> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let directory = directory.canonicalize()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("process.lock"))?;
        let name = socket_name(&directory)?;
        let files = files
            .iter()
            .map(std::path::absolute)
            .collect::<std::io::Result<Vec<_>>>()?;
        let body = serde_json::to_vec(&files)?;
        if body.len() > MAX_REQUEST {
            bail!("한 번에 전달할 파일 목록이 너무 큽니다");
        }
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let started = Instant::now();
        loop {
            match lock.try_lock() {
                Ok(()) => return Self::listen(lock, runtime, name).map(Some),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
            if runtime.block_on(Self::forward(name.clone(), &body)).is_ok() {
                return Ok(None);
            }
            // Only retry during startup/handoff; the idle listener and UI never poll.
            if started.elapsed() >= STARTUP_TIMEOUT {
                bail!("기존 EdolWave 창에 파일을 전달하지 못했습니다. 잠시 후 다시 시도하세요");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn listen(lock: File, runtime: Runtime, name: Name<'static>) -> Result<Primary> {
        let options = ListenerOptions::new()
            .name(name)
            // The file lock proves no previous owner is alive; reclaim stale Unix sockets.
            .try_overwrite(true)
            .max_spin_time(IO_TIMEOUT);
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::{
                local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
            };
            // Restrict the pipe to its owner. Interprocess also rejects remote clients.
            let descriptor =
                SecurityDescriptor::deserialize(widestring::u16cstr!("D:P(A;;GA;;;OW)"))?;
            options.security_descriptor(descriptor)
        };
        let listener = {
            let _entered = runtime.enter();
            options.create_tokio()?
        };
        let (sender, receiver) = async_channel::bounded(64);
        #[cfg(target_os = "macos")]
        let requests = sender.clone();
        let (shutdown, stopped) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("instance".into())
            .spawn(move || {
                runtime.block_on(async {
                    tokio::select! {
                        _ = stopped => {},
                        result = Self::serve(listener, sender) => {
                            if let Err(error) = result {
                                eprintln!("Instance listener: {error:#}");
                            }
                        },
                    }
                });
            })?;
        Ok((
            Self {
                _lock: lock,
                shutdown: Some(shutdown),
                worker: Some(worker),
                #[cfg(target_os = "macos")]
                requests,
            },
            receiver,
        ))
    }

    async fn serve(listener: Listener, sender: Sender<Vec<PathBuf>>) -> Result<()> {
        loop {
            let stream = listener.accept().await?;
            // Bound the whole request, including a client that connects but sends nothing.
            match timeout(IO_TIMEOUT, Self::receive(stream, &sender)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("Instance handoff: {error:#}"),
                Err(_) => eprintln!("Instance handoff timed out"),
            }
        }
    }

    async fn receive(mut stream: Stream, sender: &Sender<Vec<PathBuf>>) -> Result<()> {
        let length = stream.read_u32_le().await? as usize;
        if length > MAX_REQUEST {
            bail!("Instance request is too large");
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await?;
        let files = serde_json::from_slice(&body)?;
        sender.send(files).await?;
        stream.write_all(&[1]).await?;
        Ok(())
    }

    async fn forward(name: Name<'_>, body: &[u8]) -> Result<()> {
        timeout(IO_TIMEOUT, async {
            let mut stream = ConnectOptions::new()
                .name(name)
                .wait_mode(interprocess::ConnectWaitMode::Timeout(IO_TIMEOUT))
                .connect_tokio()
                .await?;
            stream.write_u32_le(body.len() as u32).await?;
            stream.write_all(body).await?;
            if stream.read_u8().await? != 1 {
                bail!("기존 창에서 파일 전달을 확인하지 못했습니다");
            }
            Ok(())
        })
        .await?
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use uuid::Uuid;

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            // macOS has a long temporary-directory prefix and a short Unix socket path limit.
            Self(std::env::temp_dir().join(format!("ew-{:016x}", Uuid::new_v4().as_u128() as u64)))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join("process.lock"));
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn forwards_unicode_relative_paths_and_empty_activation() {
        let directory = Directory::new();
        let (_primary, requests) = Instance::acquire_in(&directory.0, &[]).unwrap().unwrap();
        let files = vec![
            PathBuf::from("한글 음악/first song.wav"),
            PathBuf::from("next.flac"),
        ];
        assert!(
            Instance::acquire_in(&directory.0, &files)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            requests.recv_blocking().unwrap(),
            files
                .iter()
                .map(std::path::absolute)
                .collect::<std::io::Result<Vec<_>>>()
                .unwrap()
        );
        assert!(Instance::acquire_in(&directory.0, &[]).unwrap().is_none());
        assert!(requests.recv_blocking().unwrap().is_empty());
    }

    #[test]
    fn releases_lock_and_socket_after_exit() {
        let directory = Directory::new();
        let (primary, _) = Instance::acquire_in(&directory.0, &[]).unwrap().unwrap();
        drop(primary);
        let (_replacement, _) = Instance::acquire_in(&directory.0, &[]).unwrap().unwrap();
        assert!(!directory.0.join("endpoint.json").exists());
    }

    #[test]
    fn simultaneous_launches_elect_one_primary() {
        let directory = Directory::new();
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|index| {
                let path = directory.0.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    Instance::acquire_in(&path, &[PathBuf::from(format!("track-{index}.wav"))])
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_some()).count(), 1);
        let (_, requests) = results.iter().flatten().next().unwrap();
        assert_eq!(requests.recv_blocking().unwrap().len(), 1);
    }

    #[test]
    fn stalled_and_invalid_clients_do_not_break_handoff_or_shutdown() {
        let directory = Directory::new();
        let (primary, requests) = Instance::acquire_in(&directory.0, &[]).unwrap().unwrap();
        let name = socket_name(&directory.0.canonicalize().unwrap()).unwrap();
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let mut stalled = Stream::connect(name.clone()).await.unwrap();
            // A subsequent valid launch still reaches the UI after a stalled client's timeout.
            let body = serde_json::to_vec(&vec![PathBuf::from("recovered.wav")]).unwrap();
            assert!(
                timeout(Duration::from_secs(2), stalled.read_u8())
                    .await
                    .unwrap()
                    .is_err()
            );
            Instance::forward(name.clone(), &body).await.unwrap();
            assert_eq!(
                requests.recv().await.unwrap(),
                vec![PathBuf::from("recovered.wav")]
            );
            drop(stalled);

            let mut invalid = Stream::connect(name.clone()).await.unwrap();
            invalid
                .write_u32_le((MAX_REQUEST + 1) as u32)
                .await
                .unwrap();
            assert!(invalid.read_u8().await.is_err());
            assert!(requests.try_recv().is_err());

            // Shutdown cancels an outstanding receive instead of waiting for its timeout.
            let _stalled = Stream::connect(name).await.unwrap();
            let started = Instant::now();
            drop(primary);
            assert!(started.elapsed() < IO_TIMEOUT);
        });
    }
}
