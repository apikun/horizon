#!/usr/bin/env python3
"""Builds Horizon for Android: the engine with cargo-ndk, then the APK with Gradle.

    python3 tools/build_android.py                   # debug APK
    python3 tools/build_android.py --release         # optimized APK (debug-signed)
    python3 tools/build_android.py --resources DIR   # also pack prepared resources

Prerequisites (once):
    cargo install cargo-ndk --locked --version 4.1.2 --version 4.1.2 --version 4.1.2
    rustup target add aarch64-linux-android
    ANDROID_NDK_HOME pointing at an NDK (r27 or newer), and either the
    Gradle wrapper or a `gradle` on the PATH (8.11 or newer), with an
    Android SDK (command-line tools are enough; Gradle fetches the rest).

`--resources DIR` packs the folder the launcher's `horizon-prepare` built
(`.local/horizon-client/26.3`) into the APK's assets as
`horizon-resources.zip`; the app extracts it into its files folder on first
start. Horizon ships none of Mojang's files: the zip is the player's own
prepared data, and `android/app/src/main/assets/` is gitignored.

The placeholder texture pack (`android/placeholder/8x8.zip`, no Mojang
files) is copied into the assets as `placeholder-pack.zip` with every
build: the app installs it as the game's first resource pack, so a
fresh install draws with placeholder textures from the start.

The APK lands in android/app/build/outputs/apk/<kind>/.
"""

import argparse
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
JNILIBS = ROOT / "android" / "app" / "src" / "main" / "jniLibs" / "arm64-v8a"
SO_NAME = "libhorizon_android.so"
PLACEHOLDER_PACK = ROOT / "android" / "placeholder" / "8x8.zip"


def pack_placeholder_pack() -> None:
    """Copies the placeholder texture pack into the APK's assets."""
    if not PLACEHOLDER_PACK.is_file():
        sys.exit(f"missing {PLACEHOLDER_PACK}: the placeholder pack is part of the repo")
    assets = ROOT / "android" / "app" / "src" / "main" / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    out = assets / "placeholder-pack.zip"
    out.write_bytes(PLACEHOLDER_PACK.read_bytes())
    print(f"placeholder pack: {out} ({out.stat().st_size} bytes)")


def run(command: list[str], cwd: Path | None = None) -> None:
    print(f"$ {' '.join(str(c) for c in command)}", flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def build_engine(release: bool) -> Path:
    # The optimized APK builds with the `fast` profile: like release it runs
    # at opt-level 3 with thin LTO off, but with more codegen units —
    # horizon-client's codegen volume otherwise exhausts 4 GiB machines
    # (see the profile.fast.package comment in Cargo.toml).
    profile = "fast" if release else "debug"
    # cargo-ndk 4 has no subcommands: `-P` is the API level and every
    # argument it does not recognize rides through to cargo in order, so
    # the package selector belongs after `build`. (cargo-ndk 3 spelled
    # the platform flag `-p` and took a subcommand; don't mix them up.)
    run([
        "cargo", "ndk", "-t", "arm64-v8a", "-P", "31",
        "build", "-p", "horizon-android", *(["--profile", "fast"] if release else []),
    ], cwd=ROOT)
    so = ROOT / "target" / "aarch64-linux-android" / profile / SO_NAME
    if not so.is_file():
        sys.exit(f"missing {so}: did cargo ndk succeed?")
    JNILIBS.mkdir(parents=True, exist_ok=True)
    (JNILIBS / SO_NAME).write_bytes(so.read_bytes())
    print(f"engine: {JNILIBS / SO_NAME}")
    return so


def pack_resources(folder: Path) -> None:
    if not folder.is_dir():
        sys.exit(f"resources folder not found: {folder}")
    assets = ROOT / "android" / "app" / "src" / "main" / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    out = assets / "horizon-resources.zip"
    print(f"resources: zipping {folder} -> {out}")
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for path in sorted(folder.rglob("*")):
            if path.is_file():
                zf.write(path, path.relative_to(folder))


def build_apk(release: bool) -> Path:
    gradlew = ROOT / "android" / "gradlew"
    gradle = [str(gradlew)] if gradlew.is_file() else ["gradle"]
    task = "assembleRelease" if release else "assembleDebug"
    run([*gradle, "-p", "android", task, "--no-daemon"], cwd=ROOT)
    kind = "release" if release else "debug"
    outputs = ROOT / "android" / "app" / "build" / "outputs" / "apk" / kind
    apks = sorted(outputs.glob("*.apk"))
    if not apks:
        sys.exit(f"no APK in {outputs}")
    return apks[-1]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--release", action="store_true", help="optimized engine and a release APK")
    parser.add_argument(
        "--resources", type=Path, metavar="DIR",
        help="pack this prepared resources folder (e.g. .local/horizon-client/26.3) into the APK",
    )
    parser.add_argument("--engine-only", action="store_true", help="skip Gradle")
    args = parser.parse_args()
    build_engine(args.release)
    pack_placeholder_pack()
    if args.resources:
        pack_resources(args.resources)
    if not args.engine_only:
        apk = build_apk(args.release)
        print(f"\nAPK: {apk}")
        print("install with: adb install -r", apk)


if __name__ == "__main__":
    main()
