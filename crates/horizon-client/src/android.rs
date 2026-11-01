//! The Android entry: what the desktop bin's `main` does up to the event
//! loop, plus the `AndroidApp` the game's winit event loop has to be built
//! with. The `horizon-android` cdylib's `android_main` calls [`entry`]; the
//! bin's `game` module is compiled into this lib on Android so `run` is
//! reachable (`lib.rs`'s `#[path]` include — a bin's modules cannot be
//! reached from another crate).
//!
//! There is no command line in an app process: the arguments `Options`
//! would have parsed live in [`ARGS`], built here from the app's folders.
#![forbid(unsafe_code)]

use android_activity::AndroidApp;
use horizon_client::{crash, logging, telemetry};

static APP: std::sync::OnceLock<AndroidApp> = std::sync::OnceLock::new();
/// The command line the game's `Options::read` parses on Android.
static ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// The `AndroidApp` NativeActivity started the game with (the event loop's).
pub fn app() -> &'static AndroidApp {
    APP.get().expect("Horizon: android entry has not run")
}

/// The game's arguments on Android, as the entry built them.
pub fn args() -> &'static [String] {
    ARGS.get()
        .map(Vec::as_slice)
        .expect("Horizon: android entry has not run")
}

/// Runs the game, the way the desktop bin's `main` does: the log and the
/// crash handler in the app's files folder, then `game::run`. `files` is
/// `Context.getFilesDir`; the game folder (saves, options.txt, packs) is
/// `files/game`, the prepared resources live in `files/resources`.
pub fn entry(app: AndroidApp, files: std::path::PathBuf) {
    let _ = APP.set(app);
    let resources = files.join("resources");
    let game_dir = files.join("game");
    let _ = ARGS.set(vec![
        "--resources".into(),
        resources.display().to_string(),
        // `--game-dir` also chdirs there, so `options.txt`, `saves/` and the
        // rest of the desktop working directory resolve into the app's data.
        "--game-dir".into(),
        game_dir.display().to_string(),
        "--fullscreen".into(),
    ]);
    // `--home` wins over `HORIZON_HOME`: logs and crash reports in the app's
    // private storage, where a bug report can fetch them.
    logging::set_game_home(files.clone());
    let home = logging::game_home();
    let _ = std::fs::create_dir_all(&home);
    telemetry::mark_start();
    logging::init(&home, "Render thread");
    log::info!("Horizon {} (Android)", crash::version());
    crash::install(home.clone(), "client");
    raise_open_file_limit();
    if let Err(error) = crate::game::run() {
        log::error!("Horizon: {error}");
        logging::flush();
        crash::report_error(&home, "client", &*error);
        // The phone sees why instead of a silent close: the error, and where
        // the log lives. Quit on the window ends the process.
        let _ = crate::game::show_startup_error(
            &game_dir,
            "Horizon can't start",
            &[
                error.to_string(),
                "If this keeps happening, send the log along: run adb logcat -s Horizon \
                 while starting the app, or read files/home/logs/latest.log."
                    .into(),
            ],
        );
        logging::flush();
        std::process::exit(horizon_client::game::exit_code::ERROR);
    }
    logging::flush();
}

/// The JVM raises its open-file limit to the hard limit as it starts; the
/// desktop bin does the same, and a phone's soft default is as low as a
/// container's.
fn raise_open_file_limit() {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let limit = getrlimit(Resource::Nofile);
    let hard = limit.maximum;
    let raised = match (limit.current, hard) {
        (Some(soft), Some(hard)) => hard > soft,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if raised {
        let to = Rlimit {
            current: hard,
            maximum: limit.maximum,
        };
        match setrlimit(Resource::Nofile, to) {
            Ok(()) => log::info!("Open file limit raised to {hard:?}"),
            Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
        }
    }
}
