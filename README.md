# EdolWave

Rust + GPUI로 만든 Windows 네이티브 오디오 플레이어입니다. 파형 탐색, 배속 조절, 반복 재생을 지원합니다.

## 실행 및 빌드

빌드 환경: Windows 10/11, Rust 1.96 이상, Visual Studio C++ Build Tools, Windows SDK.

```powershell
# 개발용 실행 (파일 경로는 선택)
cargo run --release -- "D:\Audio\confirm.wav"

# 빠른 반복 빌드
cargo build --release

# 배포용 최적화 빌드
cargo build --profile release-dist
```

실행 파일: `target/release/edolwave.exe` 또는 `target/release-dist/edolwave.exe`.

## 설치 프로그램 및 배포

`edolview-rs`와 같은 Inno Setup 기반으로 Windows x64 설치 프로그램을 만듭니다.
기본은 관리자 권한이 필요 없는 사용자별 설치이며, 시작 메뉴 바로가기와 선택적 바탕 화면 바로가기를 지원합니다.
설치 프로그램과 포터블 ZIP에는 MSVC 런타임 DLL이 포함됩니다. FFmpeg는 포함하지 않습니다.

```powershell
# 최초 한 번: 고정 버전과 SHA-256을 검증하여 로컬 빌드 도구 설치
./tools/install_inno.ps1

# 최적화 빌드 + 설치 프로그램 + 포터블 ZIP + SHA-256 생성
./tools/package.ps1

# 이미 배포용 빌드가 있다면
./tools/package.ps1 -NoBuild
```

결과물은 `target/dist/edolwave-installer-exe.exe`, `target/dist/edolwave-windows.zip`,
`target/dist/SHA256SUMS.txt`입니다. 설치 파일은 서명되지 않은 상태로 생성됩니다.

`.github/workflows/deploy.yml`은 `main`/`master` push, PR, 수동 실행에서 포맷 검사, 테스트,
배포용 빌드, 패키징과 자동 설치·제거 검증을 수행하고 Actions 아티팩트에 결과물을 저장합니다.
`v<버전>` 태그 push 시에는 체크섬 검증 후 같은 파일을 GitHub Release에 자동 게시합니다.
별도 토큰 없이 저장소의 `GITHUB_TOKEN`을 사용하며 Release 작업에만 `contents: write` 권한을 부여합니다.

버전의 기준은 `Cargo.toml`입니다. `Cargo.lock`도 함께 갱신하고 두 파일을 커밋한 뒤,
일치하는 태그를 push하세요. Windows 버전 리소스에 맞춰 `major.minor.patch` 형식만 지원합니다.
예를 들어 현재 버전 `0.1.0`의 릴리스는 다음과 같습니다.

```powershell
git tag v0.1.0
git push origin v0.1.0
```

임시 앱 아이콘의 원본은 `icon.svg`이며 `icons/icon.png`와 `icons/app.ico`를 함께 관리합니다.
실행 파일과 설치 프로그램 모두 이 ICO를 사용합니다. SVG 변경 후 ImageMagick 7이 설치된 환경에서
`./tools/generate-icons.ps1`을 실행하고 생성된 두 파일도 커밋하세요. 일반 빌드와 CI에는 ImageMagick이 필요하지 않습니다.

## 조작

- 파일 열기: 드래그 앤 드롭, 폴더 버튼 또는 `Ctrl+O`. 여러 파일을 함께 열 수 있습니다.
- 탐색: 파형 클릭·드래그, `←` / `→`로 5초 이동, `Home`으로 처음 이동.
- 재생·일시정지: `Space`. 음소거: `M`. 반복: `R`.
- 볼륨: 슬라이더·휠 또는 `↑` / `↓`로 5%씩 조절.
- 배속: 드롭다운·휠로 0.5~2배 선택. 음높이도 함께 바뀝니다.

## 지원 형식

WAV, MP3, FLAC, OGG/Vorbis, AAC, M4A/MP4, AIFF 등을 기본 지원합니다. 컨테이너 내부 코덱에 따라 지원 여부가 달라집니다.

Opus, WMA, APE 등 추가 형식은 별도로 설치한 FFmpeg를 사용합니다. FFmpeg를 `PATH`에 추가하거나 `EDOLWAVE_FFMPEG` 환경 변수에 실행 파일 경로를 지정하세요.
