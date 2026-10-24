"""Validate native Unix packages; Linux GUI smoke test runs inside Xvfb + D-Bus."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "target/dist"
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]


def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess:
    return subprocess.run(list(map(str, args)), check=True, **kwargs)


def checksums(suffix: str) -> None:
    for line in (DIST / f"SHA256SUMS-{suffix}.txt").read_text().splitlines():
        digest, name = line.split("  ", 1)
        assert hashlib.sha256((DIST / name).read_bytes()).hexdigest() == digest, name


def check_app(app: Path) -> None:
    contents = app / "Contents"
    info = plistlib.loads((contents / "Info.plist").read_bytes())
    assert info["CFBundleIdentifier"] == "dev.edoli.edolwave"
    assert info["CFBundleVersion"] == VERSION
    formats = json.loads((ROOT / "installer/audio-formats.json").read_text())
    assert info["CFBundleDocumentTypes"][0]["CFBundleTypeExtensions"] == formats["native"]
    assert (contents / "Resources/app.icns").is_file()
    binary = contents / "MacOS/edolwave"
    assert os.access(binary, os.X_OK)
    run("codesign", "--verify", "--deep", "--strict", app)
    dependencies = run("otool", "-L", binary, capture_output=True, text=True).stdout
    assert "/opt/homebrew/" not in dependencies and "/usr/local/" not in dependencies
    print(dependencies)


def test_mac(stage: Path) -> None:
    checksums("mac")
    run("ditto", "-x", "-k", DIST / "edolwave-mac.zip", stage)
    check_app(stage / "EdolWave.app")
    image = DIST / "edolwave-installer-dmg.dmg"
    attached = run("hdiutil", "attach", "-readonly", "-nobrowse", "-plist", image,
                   capture_output=True).stdout
    mount = next(Path(entry["mount-point"]) for entry in plistlib.loads(attached)["system-entities"]
                 if "mount-point" in entry)
    try:
        check_app(mount / "EdolWave.app")
        assert (mount / "Applications").is_symlink()
    finally:
        run("hdiutil", "detach", mount)
    print("PASS: macOS ZIP/DMG versions, document types, signatures and system-only dependencies")


def test_linux(stage: Path) -> None:
    checksums("linux")
    portable = stage / "portable"
    run("unzip", "-q", DIST / "edolwave-linux.zip", "-d", portable)
    assert os.access(portable / "edolwave", os.X_OK)
    image = DIST / "edolwave-installer-appimage.AppImage"
    run(image, "--appimage-extract", cwd=stage, stdout=subprocess.DEVNULL)
    assert os.access(stage / "squashfs-root/AppRun", os.X_OK)
    package = DIST / "edolwave-installer-deb.deb"
    assert run("dpkg-deb", "-f", package, "Version", capture_output=True, text=True).stdout.strip() == VERSION
    status = subprocess.run(["dpkg-query", "-W", "-f=${Status}", "edolwave"],
                            capture_output=True, text=True)
    if status.returncode == 0 and "installed" in status.stdout:
        raise RuntimeError("An EdolWave DEB is already installed; leaving it untouched")
    try:
        run("sudo", "dpkg", "--install", package)
        assert Path("/usr/bin/edolwave").is_file()
        assert Path("/usr/share/applications/dev.edoli.edolwave.desktop").is_file()
    finally:
        run("sudo", "dpkg", "--remove", "edolwave")
    assert not Path("/usr/bin/edolwave").exists()
    print("PASS: Linux checksums, ZIP permissions, AppImage extraction, DEB install/uninstall")

    env = dict(os.environ, EDOLWAVE_INSTANCE_SCOPE=f"ci-{uuid.uuid4()}", GPUI_BACKEND="X11")
    log_path = DIST / "linux-smoke.log"
    with log_path.open("wb") as log:
        primary = subprocess.Popen([str(portable / "edolwave")], env=env, stdout=log, stderr=log)
        try:
            end = time.monotonic() + 20
            while True:
                if primary.poll() is not None:
                    raise RuntimeError(f"Player exited: {log_path.read_text(errors='replace')}")
                windows = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(primary.pid)],
                                         capture_output=True, text=True)
                if windows.returncode == 0 and windows.stdout.strip():
                    break
                if time.monotonic() >= end:
                    raise TimeoutError(f"No player window: {log_path.read_text(errors='replace')}")
                time.sleep(.05)
            run(portable / "edolwave", env=env, stdout=log, stderr=log, timeout=10)
            assert primary.poll() is None
            after = run("xdotool", "search", "--onlyvisible", "--pid", primary.pid,
                        capture_output=True, text=True).stdout
            assert after.splitlines() == windows.stdout.splitlines()
            print("PASS: Linux portable startup and single-instance activation under Xvfb/Vulkan")
        finally:
            primary.terminate()
            try:
                primary.wait(timeout=5)
            except subprocess.TimeoutExpired:
                primary.kill()
                primary.wait()


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="package-test-", dir=ROOT / "target") as temporary:
        if sys.platform == "darwin":
            test_mac(Path(temporary))
        elif sys.platform == "linux":
            test_linux(Path(temporary))
        else:
            raise RuntimeError("Use tools/test-installer.ps1 on Windows")


if __name__ == "__main__":
    main()
