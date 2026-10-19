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

## 조작

- 파일 열기: 드래그 앤 드롭, 폴더 버튼 또는 `Ctrl+O`. 여러 파일을 함께 열 수 있습니다.
- 탐색: 파형 클릭·드래그, `←` / `→`로 5초 이동, `Home`으로 처음 이동.
- 재생·일시정지: `Space`. 음소거: `M`. 반복: `R`.
- 볼륨: 슬라이더·휠 또는 `↑` / `↓`로 5%씩 조절.
- 배속: 드롭다운·휠로 0.5~2배 선택. 음높이도 함께 바뀝니다.

## 지원 형식

WAV, MP3, FLAC, OGG/Vorbis, AAC, M4A/MP4, AIFF 등을 기본 지원합니다. 컨테이너 내부 코덱에 따라 지원 여부가 달라집니다.

Opus, WMA, APE 등 추가 형식은 별도로 설치한 FFmpeg를 사용합니다. FFmpeg를 `PATH`에 추가하거나 `EDOLWAVE_FFMPEG` 환경 변수에 실행 파일 경로를 지정하세요.
