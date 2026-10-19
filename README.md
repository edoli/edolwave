# EdolWave

Rust + GPUI로 만든 네이티브 오디오 플레이어입니다. 파형 탐색, 배속 조절, 반복 재생을 지원합니다.

## 배포 파일

GitHub Release의 `v<버전>` 태그에서 다음 파일을 제공합니다. 태그 버전은 `Cargo.toml`과 일치해야 합니다.

| 플랫폼 | 아키텍처 | 포터블 / 앱 | 설치 패키지 |
|---|---|---|---|
| Windows 10/11 | x64 | `edolwave-windows.zip` | `edolwave-installer-exe.exe` |
| macOS 12 이상 | Apple Silicon (arm64) | `edolwave-mac.zip` | `edolwave-installer-dmg.dmg` |
| Linux (Ubuntu 24.04 기준) | x64 | `edolwave-linux.zip` | `edolwave-installer-deb.deb`, `edolwave-installer-appimage.AppImage` |

macOS는 DMG에서 `EdolWave.app`을 Applications로 복사합니다. 앱에는 임시 서명이 적용되며 Apple 공증은 포함하지 않습니다.
Linux ZIP은 압축을 푼 뒤 `AppRun`으로 실행합니다. Vulkan 지원 드라이버가 필요하며, AppImage는 FUSE가 없으면 `APPIMAGE_EXTRACT_AND_RUN=1`로 실행할 수 있습니다.
각 OS의 파일 열기 목록에 지원 오디오 형식을 등록하며 기존 기본 앱을 강제로 변경하지 않습니다.
릴리스에는 모든 배포 파일의 `SHA256SUMS.txt`가 포함됩니다. 일반 브랜치 빌드도 GitHub Actions 아티팩트로 패키지를 제공합니다.

## 실행 및 빌드

공통: Rust 1.96 이상. 패키징 스크립트에는 Python 3.11 이상이 필요합니다.

- Windows: Visual Studio C++ Build Tools, Windows SDK. 설치 프로그램 생성에는 Inno Setup이 필요합니다.
- macOS: Xcode와 Metal 컴파일러. 필요하면 `xcodebuild -downloadComponent MetalToolchain`으로 설치합니다.
- Linux: C/C++ 컴파일러, Clang, CMake, pkg-config, ALSA·Fontconfig·FreeType·Wayland·X11/XKB 개발 라이브러리와 Vulkan 로더가 필요합니다. CI의 설치 목록은 `.github/workflows/deploy.yml`에 있습니다.

```powershell
# 개발용 실행 (파일 경로는 선택)
cargo run --release -- "D:\Audio\confirm.wav"

# 빠른 반복 빌드
cargo build --release

# 배포용 최적화 빌드
cargo build --profile release-dist
```

실행 파일: `target/release/edolwave.exe` 또는 `target/release-dist/edolwave.exe`.
macOS와 Linux에서는 `.exe` 확장자가 없습니다.

패키징: Windows는 `./tools/package.ps1`, macOS/Linux는 `python3 tools/package-unix.py`를 실행합니다.
이미 빌드했다면 각각 `-NoBuild`, `--no-build`로 빌드를 생략할 수 있습니다.
결과는 `target/dist`에 생성됩니다. Linux 패키징에는 `desktop-file-utils`, `dpkg-dev`, `patchelf`, `squashfs-tools`, `file`, `unzip`도 필요합니다.

## 조작

- 파일 열기: 드래그 앤 드롭, 폴더 버튼 또는 `Ctrl+O` (macOS는 `Cmd+O`). 여러 파일을 함께 열 수 있습니다.
- 실행 중 다른 파일을 열면 기존 창에서 재생합니다. 파일 없이 다시 실행하면 기존 창을 활성화합니다.
- 탐색: 파형 클릭·드래그, `←` / `→`로 5초 이동, `Home`으로 처음 이동.
- 재생·일시정지: `Space`. 음소거: `M`. 반복: `R`.
- 볼륨: 슬라이더·휠 또는 `↑` / `↓`로 5%씩 조절.
- 배속: 드롭다운·휠로 0.5~2배 선택. 음높이도 함께 바뀝니다.

## 지원 형식

WAV, MP3, FLAC, OGG/Vorbis, AAC, M4A/MP4, AIFF 등을 기본 지원합니다. 컨테이너 내부 코덱에 따라 지원 여부가 달라집니다.
