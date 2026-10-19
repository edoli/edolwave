"""Build native macOS/Linux distributions from target/release-dist/edolwave.

Uses the same Cargo version, icon and audio-extension manifest as Windows.
Run with Python 3.11+ on the platform being packaged; no Python packages needed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "target/dist"
APP_ID = "dev.edoli.edolwave"
LINUXDEPLOY_URL = (
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/"
    "1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
)
LINUXDEPLOY_SHA256 = "c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"
MIME_TYPES = (
    "audio/x-wav", "audio/wav", "audio/vnd.wave", "audio/mpeg", "audio/flac",
    "audio/ogg", "audio/x-vorbis+ogg", "audio/aac", "audio/x-aac", "audio/mp4",
    "audio/x-m4a", "audio/x-m4b", "audio/x-aiff", "audio/x-caf", "audio/x-matroska",
)


def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
    print("+", " ".join(map(str, args)), flush=True)
    try:
        return subprocess.run(list(map(str, args)), check=True, **kwargs)
    except subprocess.CalledProcessError as error:
        if error.stdout:
            print(error.stdout, file=sys.stderr)
        if error.stderr:
            print(error.stderr, file=sys.stderr)
        raise


def version() -> str:
    value = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError(f"Unsupported package version: {value}")
    if os.environ.get("GITHUB_REF_TYPE") == "tag":
        if os.environ.get("GITHUB_REF_NAME") != f"v{value}":
            raise ValueError("Git tag does not match Cargo.toml version")
    return value


def desktop_entry() -> str:
    return (
        "[Desktop Entry]\nType=Application\nName=EdolWave\n"
        "Comment=Waveform audio player\nExec=edolwave %F\nIcon=edolwave\n"
        "Terminal=false\nCategories=AudioVideo;Audio;Player;\n"
        f"StartupWMClass={APP_ID}\nMimeType={';'.join(MIME_TYPES)};\n"
    )


def app_plist(build_version: str) -> dict:
    extensions = json.loads((ROOT / "installer/audio-formats.json").read_text())["native"]
    return {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "EdolWave",
        "CFBundleName": "EdolWave",
        "CFBundleIdentifier": APP_ID,
        "CFBundleExecutable": "edolwave",
        "CFBundlePackageType": "APPL",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleIconFile": "app.icns",
        "CFBundleShortVersionString": build_version,
        "CFBundleVersion": build_version,
        "LSMinimumSystemVersion": "12.0",
        "NSHighResolutionCapable": True,
        "CFBundleDocumentTypes": [{
            "CFBundleTypeName": "Supported audio files",
            "CFBundleTypeExtensions": extensions,
            "CFBundleTypeRole": "Viewer",
            "LSHandlerRank": "Alternate",
        }],
    }


def write_executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8", newline="\n")
    path.chmod(0o755)


def copy_binary(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    destination.chmod(0o755)


def zip_tree(source: Path, destination: Path) -> None:
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(source.rglob("*")):
            name = path.relative_to(source).as_posix()
            if path.is_symlink():
                entry = zipfile.ZipInfo(name)
                entry.create_system = 3
                entry.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(entry, os.readlink(path))
            elif path.is_file():
                archive.write(path, name)


def package_mac(binary: Path, stage: Path, build_version: str) -> list[Path]:
    app = stage / "EdolWave.app"
    contents = app / "Contents"
    resources = contents / "Resources"
    resources.mkdir(parents=True)
    copy_binary(binary, contents / "MacOS/edolwave")
    for name in ("README.md", "LICENSE"):
        shutil.copy2(ROOT / name, resources / name)
    (contents / "Info.plist").write_bytes(plistlib.dumps(app_plist(build_version)))
    iconset = stage / "app.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            suffix = "@2x" if scale == 2 else ""
            run("sips", "-z", size * scale, size * scale, ROOT / "icons/icon.png",
                "--out", iconset / f"icon_{size}x{size}{suffix}.png")
    run("iconutil", "-c", "icns", iconset, "-o", resources / "app.icns")
    run("codesign", "--force", "--deep", "--sign", "-", app)
    run("codesign", "--verify", "--deep", "--strict", app)
    archive = DIST / "edolwave-mac.zip"
    run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
    image_root = stage / "dmg"
    image_root.mkdir()
    shutil.copytree(app, image_root / app.name, symlinks=True)
    (image_root / "Applications").symlink_to("/Applications")
    image = DIST / "edolwave-installer-dmg.dmg"
    run("hdiutil", "create", "-volname", "EdolWave", "-srcfolder", image_root,
        "-ov", "-format", "UDZO", image)
    run("hdiutil", "verify", image)
    return [archive, image]


def linuxdeploy() -> Path:
    tool = ROOT / "target/tools/linuxdeploy-x86_64.AppImage"
    tool.parent.mkdir(parents=True, exist_ok=True)
    if not tool.exists() or hashlib.sha256(tool.read_bytes()).hexdigest() != LINUXDEPLOY_SHA256:
        with urllib.request.urlopen(LINUXDEPLOY_URL) as response:
            data = response.read()
        if hashlib.sha256(data).hexdigest() != LINUXDEPLOY_SHA256:
            raise ValueError("linuxdeploy checksum mismatch")
        tool.write_bytes(data)
    tool.chmod(0o755)
    return tool


def package_deb(binary: Path, stage: Path, desktop: Path, build_version: str) -> Path:
    root = stage / "deb"
    copy_binary(binary, root / "usr/bin/edolwave")
    applications = root / "usr/share/applications"
    applications.mkdir(parents=True)
    shutil.copy2(desktop, applications / desktop.name)
    icon = root / "usr/share/icons/hicolor/512x512/apps/edolwave.png"
    icon.parent.mkdir(parents=True)
    shutil.copy2(ROOT / "icons/icon.png", icon)
    documentation = root / "usr/share/doc/edolwave"
    documentation.mkdir(parents=True)
    for name in ("README.md", "LICENSE"):
        shutil.copy2(ROOT / name, documentation / name)
    # Resolve runtime dependencies from the actual ELF binary, like cargo-deb's $auto.
    debian = stage / "debian"
    debian.mkdir()
    (debian / "control").write_text(
        "Source: edolwave\nSection: sound\nPriority: optional\n"
        "Maintainer: Daniel Jeon <edoli.mia@gmail.com>\n\n"
        "Package: edolwave\nArchitecture: any\nDescription: Waveform audio player\n",
        encoding="utf-8",
    )
    dependencies = run("dpkg-shlibdeps", "-O", f"-e{binary}", cwd=stage,
                       capture_output=True, text=True).stdout.strip()
    dependencies = next(line.removeprefix("shlibs:Depends=") for line in dependencies.splitlines()
                        if line.startswith("shlibs:Depends="))
    # GPUI loads these at runtime, so ELF dependency scanning cannot discover them.
    dependencies += ", libvulkan1, libwayland-client0"
    control = root / "DEBIAN"
    control.mkdir()
    (control / "control").write_text(
        f"Package: edolwave\nVersion: {build_version}\nArchitecture: amd64\n"
        "Section: sound\nPriority: optional\n"
        "Maintainer: Daniel Jeon <edoli.mia@gmail.com>\n"
        f"Depends: {dependencies}\nRecommends: xdg-desktop-portal, fonts-noto-cjk\n"
        "Description: Native waveform audio player\n"
        " Audio playback, waveform navigation, playback speed and repeat controls.\n",
        encoding="utf-8",
    )
    for event in ("postinst", "postrm"):
        write_executable(control / event,
            "#!/bin/sh\nset -e\n"
            "if command -v update-desktop-database >/dev/null 2>&1; then\n"
            "  update-desktop-database -q /usr/share/applications || true\nfi\n"
            "if command -v gtk-update-icon-cache >/dev/null 2>&1; then\n"
            "  gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true\nfi\n")
    destination = DIST / "edolwave-installer-deb.deb"
    run("dpkg-deb", "--root-owner-group", "--build", root, destination)
    return destination


def package_linux(binary: Path, stage: Path, build_version: str) -> list[Path]:
    if platform.machine() != "x86_64":
        raise ValueError("Linux packages currently target x86_64")
    desktop = stage / f"{APP_ID}.desktop"
    desktop.write_text(desktop_entry(), encoding="utf-8", newline="\n")
    run("desktop-file-validate", desktop)
    deb = package_deb(binary, stage, desktop, build_version)
    appdir = stage / "AppDir"
    copy_binary(binary, appdir / "usr/bin/edolwave")
    icon = stage / "edolwave.png"
    shutil.copy2(ROOT / "icons/icon.png", icon)
    write_executable(appdir / "AppRun",
        '#!/bin/sh\nAPPDIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\n'
        'export LD_LIBRARY_PATH="$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n'
        'exec "$APPDIR/usr/bin/edolwave" "$@"\n')
    for name in ("README.md", "LICENSE"):
        shutil.copy2(ROOT / name, appdir / name)
    image = DIST / "edolwave-installer-appimage.AppImage"
    env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN="1", VERSION=build_version,
               ARCH="x86_64", LDAI_OUTPUT=str(image))
    run(linuxdeploy(), "--appdir", appdir, "--executable", appdir / "usr/bin/edolwave",
        "--desktop-file", desktop, "--icon-file", icon, "--output", "appimage",
        env=env, cwd=stage)
    if not image.is_file():
        raise FileNotFoundError(f"AppImage output missing: {image}")
    image.chmod(0o755)
    archive = DIST / "edolwave-linux.zip"
    zip_tree(appdir, archive)
    return [archive, deb, image]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux"):
        parser.error("Run this script on macOS or Linux")
    build_version = version()
    if not args.no_build:
        run("cargo", "build", "--profile", "release-dist", "--locked", cwd=ROOT)
    binary = ROOT / "target/release-dist/edolwave"
    if not binary.is_file():
        raise FileNotFoundError(binary)
    DIST.mkdir(parents=True, exist_ok=True)
    # TemporaryDirectory owns only a newly-created directory inside target.
    with tempfile.TemporaryDirectory(prefix="package-", dir=ROOT / "target") as temporary:
        stage = Path(temporary)
        if sys.platform == "darwin":
            outputs = package_mac(binary, stage, build_version)
            suffix = "mac"
        else:
            outputs = package_linux(binary, stage, build_version)
            suffix = "linux"
    checksums = DIST / f"SHA256SUMS-{suffix}.txt"
    checksums.write_text("".join(
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n" for path in outputs
    ), encoding="ascii", newline="\n")
    for path in [*outputs, checksums]:
        print(f"{path.name}: {path.stat().st_size:,} bytes")


if __name__ == "__main__":
    main()
