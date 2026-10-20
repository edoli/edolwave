# EdolWave

Rust + GPUI로 만든 Windows·macOS·Linux 네이티브 오디오 플레이어입니다. 파형 탐색, 배속 조절, 반복 재생을 지원합니다.

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
- 배속: 드롭다운·휠로 0.5~2배 선택. 음높이도 함께 바뀝니다.

## 지원 형식

WAV, MP3, FLAC, OGG/Vorbis, AAC, M4A/MP4, AIFF 등을 기본 지원합니다. 컨테이너 내부 코덱에 따라 지원 여부가 달라집니다.

## 업데이트

앱을 시작하면 GitHub의 최신 정식 릴리스를 백그라운드에서 확인합니다. 설정을 열 때도 이전 확인 결과가 최신 버전이거나 오류이면 다시 확인합니다. 이미 업데이트를 찾았거나 다운로드 중이면 해당 상태를 유지합니다. 주기적인 확인 타이머와 수동 확인 버튼은 없습니다. 업데이트가 있으면 정보 버튼 옆 설정 버튼에 파란 점이 표시됩니다. 설정에서 현재 버전과 업데이트 상태를 확인하고, 설치 파일을 다운로드할 수 있습니다. 네트워크 오류나 접근 권한 오류는 최신 버전 상태와 구분해서 표시합니다.

다운로드는 크기와 GitHub 릴리스의 SHA-256으로 검증합니다. Windows는 `설치하고 종료`를 누르면 설치 프로그램을 실행하고 앱을 종료합니다. 기존 설치가 감지되면 같은 폴더로 설치합니다. macOS는 DMG에서 앱을 복사하고, Linux DEB는 패키지 설치 화면을 사용합니다. AppImage는 다운로드 폴더를 열어 직접 기존 파일을 교체합니다.

비공개 저장소를 이용하는 개발 환경에서는 `EDOLWAVE_GITHUB_TOKEN` 환경 변수에 해당 저장소의 Contents 읽기 권한이 있는 토큰을 지정해야 합니다. 토큰을 앱 바이너리에 포함하거나 저장하지 않습니다. 일반 사용자에게 별도 인증 없이 업데이트를 제공하려면 공개 릴리스가 필요합니다.
