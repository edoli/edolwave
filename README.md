# EdolWave

Rust + GPUI로 만든 Windows 네이티브 오디오 플레이어입니다. 파형 탐색, 배속 조절, 반복 재생을 지원합니다.

## 실행 및 빌드

공통: Rust 1.96 이상.

- Windows: Visual Studio C++ Build Tools, Windows SDK.
- macOS: Xcode와 Metal 컴파일러. 필요하면 `xcodebuild -downloadComponent MetalToolchain`으로 설치합니다.
- Linux: C/C++ 컴파일러, Clang, CMake, pkg-config, ALSA·Fontconfig·FreeType·Wayland·X11/XKB 개발 라이브러리와 Vulkan 로더가 필요합니다.

```powershell
# 실행 예제
cargo run --release -- "D:\Audio\confirm.wav"
```

실행 파일: `target/release/edolwave.exe` 또는 `target/release-dist/edolwave.exe`.

## 조작

- 파일 열기: 드래그 앤 드롭, 폴더 버튼 또는 `Ctrl+O`. 여러 파일을 함께 열 수 있습니다.
- 실행 중 다른 파일을 열면 기존 창에서 재생합니다. 파일 없이 다시 실행하면 기존 창을 활성화합니다.
- 탐색: 파형 클릭·드래그, `←` / `→`로 5초 이동, `Home`으로 처음 이동.
- 재생·일시정지: `Space`. 음소거: `M`. 반복: `R`.
- 볼륨: 슬라이더·휠 또는 `↑` / `↓`로 5%씩 조절.
- 테마: 설정에서 밝은 테마·다크 테마·시스템 설정을 선택합니다(기본값: 시스템 설정). 선택은 자동 저장되며, 시스템 설정에서는 운영체제 테마 변경을 반영합니다.
- 업데이트: 설정의 업데이트 버튼을 한 번 누르면 다운로드·SHA-256 검증·설치 후 자동으로 다시 실행합니다. Windows 설치형은 기존 설치 범위와 경로를 유지하고, 포터블은 ZIP으로 현재 파일을 교체합니다. 필요한 경우 운영체제 권한 확인이 표시됩니다.
- 배속: 드롭다운·휠로 0.5~2배 선택. 음높이도 함께 바뀝니다.

## 지원 형식

WAV, MP3, FLAC, OGG/Vorbis, AAC, M4A/MP4, AIFF 등을 기본 지원합니다. 컨테이너 내부 코덱에 따라 지원 여부가 달라집니다.

Windows 설치 화면의 파일 연결 확장자는 기본적으로 접혀 있습니다. `Show file formats`를 누르면 4열 목록에서 선택할 수 있습니다. 이전 선택과 `/TASKS` 지정은 그대로 적용됩니다.
