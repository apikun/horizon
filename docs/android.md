# Horizon on Android

The game on a phone or tablet, as one NativeActivity: the whole engine —
world generation, the server thread, rendering, sound — is the same Rust the
desktop builds run, cross-compiled for arm64 and driven by touch. Nothing
about the simulation is different; only the window, the input and the
packaging are.

Status and gaps live at the end of this page. Horizon ships none of Mojang's
files on any platform, Android included: the game needs the prepared
resources (`horizon-prepare`) from your own Minecraft: Java Edition, copied
onto the device (see [Resources](#resources)).

## Install

Two ways to get an APK:

- **CI (nothing to install):** open **Actions → android** on the repository,
  run the workflow (`workflow_dispatch`), and download the `horizon-android`
  artifact. It is a release build without resources.
- **Locally:** [Build](#build) below.

Then:

```sh
adb install -r horizon.apk
```

or copy the APK to the device and open it. Android 12 (API 31) or newer,
arm64, and a GPU with working Vulkan (or GLES 3) drivers.

## Build

Once:

```sh
cargo install cargo-ndk --locked --version 4.1.2
rustup target add aarch64-linux-android
# ANDROID_NDK_HOME points at an NDK r27+; a Java 21 JDK and an Android SDK
# (command-line tools suffice) for the packaging; a `gradle` 8.11+ on PATH.
```

Every time:

```sh
python3 tools/build_android.py                # debug
python3 tools/build_android.py --release      # optimized
python3 tools/build_android.py --release \
    --resources .local/horizon-client/26.3    # with your prepared resources bundled
```

The script cross-compiles `crates/horizon-android` (the NativeActivity glue)
with cargo-ndk, drops `libhorizon_android.so` into the Gradle project's
`jniLibs`, and runs Gradle. The APK lands in
`android/app/build/outputs/apk/<kind>/`.

## Resources

The app looks for the prepared resources in its private storage, under
`files/resources/`. Three ways to put them there:

1. **Bundled APK (easiest):** build with `tools/build_android.py
   --resources .local/horizon-client/26.3`. The prepared folder is zipped
   into the APK's assets as `horizon-resources.zip` and extracted into the
   app's files folder on first start. The zip stays on your machine
   (`android/app/src/main/assets/` is gitignored) — the APK must not be
   shared with people who have no right to Minecraft's files.
2. **Import folder:** build without resources, then push a zip you made of
   the prepared folder:
   `adb push horizon-resources.zip /data/local/tmp/ && adb shell run-as
   dev.playhorizon.android mkdir -p files/import && adb shell run-as
   dev.playhorizon.android cp /data/local/tmp/horizon-resources.zip
   files/import/`, then start the app once. A bundle in `files/import/` is
   extracted over `files/resources/` at startup, so pushing a fresh one
   updates the game's data.
3. **Direct copy (root or `run-as` on a debug build):** copy
   `.local/horizon-client/26.3` to `files/resources/` directly.

Saves, `options.txt`, resource packs and mods live in `files/game/` —
`adb shell run-as dev.playhorizon.android` reaches them on a debug build.
`files/home/` holds `logs/latest.log` and `crash-reports/`, exactly as on
the desktop.

## Controls

Touch follows the scheme players know from the Bedrock edition, routed
through the game's own input so keybinds, sensitivity and toggle options
apply unchanged:

| Gesture / button | Action |
|---|---|
| Left part of the screen, drag | Virtual joystick: walk; push far to sprint |
| Right part of the screen, drag | Look around (the right stick's sensitivity) |
| Tap on the look surface | Right click: use / place / interact |
| Hold on the look surface | Left click held: mine / attack |
| **A** (bottom right) | Jump (hold) |
| **S** (above A) | Sneak (tap to toggle, like Bedrock's) |
| **Q** (left of A) | Drop item |
| **E** (top right) | Inventory |
| **M** (top left) | Pause / back |
| Over menus | Touch is the cursor, tap is a click |

The overlay buttons press the virtual pad's buttons, so they follow the
controller bindings where those are rebound. A physical gamepad plugged into
the device works too (via the OS, as a pad).

## What works, what does not (yet)

Works: the title screen, single player (server-thread mode), resource packs
from `files/game/resourcepacks/`, LAN and direct-IP multiplayer, sound
(cpal's AAudio output), suspend and resume (the renderer rebuilds, the world
and its server thread keep their state).

Known gaps, roughly in the order someone will hit them:

- **Text entry**: chat and text fields have no soft-keyboard integration
  yet (NativeActivity's IME path in winit is the missing piece). Type on a
  gamepad or play without chat.
- **Scrolling in menus**: a touch drags the cursor but does not scroll
  lists (options, the world list). Use the drag bars where a screen has
  them, or a gamepad.
- **Suspend cost**: the first frame after resume rebuilds the wgpu device
  and pipelines (a few seconds).
- **Performance** is untested across devices; start with a small render
  distance in the video settings.
- The no-resources screen names the desktop launcher; on Android follow
  [Resources](#resources) instead.
- The launcher has no Android build: instances, modpacks and accounts are
  managed on a computer, or by hand in `files/game/`.

## How it fits together

- `crates/horizon-android` — the cdylib NativeActivity loads: `android_main`
  sets up the allocator, extracts a bundled resource zip, and hands the
  `AndroidApp` to `horizon_client::android::entry`, which does what the
  desktop bin's `main` does (log, crash handler) and runs `game::run`.
- `crates/horizon-client/src/android.rs` — that entry, plus the arguments
  the game would have parsed from the command line (`--resources`,
  `--game-dir`, `--fullscreen`), built from the app's folders.
- `crates/horizon-client/src/game/touch.rs` — the touch layer: it feeds the
  virtual pad (`pad.rs`) and synthesizes the mouse events the screens
  already eat; it draws the overlay through the HUD batch.
- Lifecycle: winit's `Suspended` drops the window and renderer on Android;
  `Resumed` recreates them through the paths `resumed` already had.
- `android/` — the Gradle project (a manifest, an icon, no Java code) that
  packages `libhorizon_android.so` into an APK; `tools/build_android.py`
  drives the whole build; `.github/workflows/android.yml` builds the APK on
  GitHub's runners.
