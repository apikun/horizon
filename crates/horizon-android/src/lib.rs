//! Horizon on Android: the shared library NativeActivity loads.
//!
//! The desktop game starts from `main` in the `horizon-client` bin; an app
//! process starts here instead, in `android_main`. This file does what that
//! `main` does before the game loop — the allocator, the mod API mark, the
//! game folder, the log and the crash handler — then hands over to
//! `horizon_client::android::entry`, which owns the `AndroidApp` winit's
//! event loop needs and runs `game::run`.
//!
//! None of Mojang's files ship in the APK: a bundle of the player's own
//! prepared resources (`tools/build_android.py --resources`) may be packed
//! into the APK's assets as `horizon-resources.zip`, and is extracted into
//! the app's files folder here on first run, before the game looks for it.
//!
//! On the desktop this crate is an empty stub (CI clippy's `--workspace`
//! still builds it): everything below is `cfg(target_os = "android")`.
//!
//! No `#![forbid(unsafe_code)]` here: the unsafe boundary of the port is
//! this file's `android_main` export (the linker must find the symbol) —
//! the game itself stays forbid(unsafe), as the desktop bin is.

#[cfg(target_os = "android")]
mod android {
    use android_activity::AndroidApp;

    #[global_allocator]
    static ALLOCATOR: horizon_threads::Allocator = horizon_threads::Allocator;

    /// The mod API level this game serves, kept in the program for the
    /// launcher to read without running it (`horizon_mods::api_level::MARK`),
    /// as the desktop bin keeps it.
    static MOD_API_LEVEL: [u8; 27] = horizon_mods::api_level::MARK;

    /// The name of the asset that holds the prepared resources, and the
    /// folder under the app's files dir they are extracted into (the game's
    /// `--resources` root).
    const RESOURCE_ASSET: &str = "horizon-resources.zip";
    const RESOURCES_DIR: &str = "resources";

    #[unsafe(no_mangle)]
    fn android_main(app: AndroidApp) {
        horizon_threads::configure_allocator();
        std::hint::black_box(&MOD_API_LEVEL);

        // The game folder: the app's private files dir. Everything the
        // desktop run keeps in its working directory (saves, options.txt,
        // packs, logs) lives under it.
        let files = files_dir(&app);
        let resources = files.join(RESOURCES_DIR);
        let bundle = files.join("import").join(RESOURCE_ASSET);
        let bundled = std::fs::metadata(&bundle)
            .map(|meta| meta.len() > 0)
            .unwrap_or(false);
        // A bundle dropped into `files/import/` (or packed into the APK)
        // replaces the extracted resources: the player updates the game's
        // data by pushing a fresh zip and restarting.
        let zip_source: Option<(std::io::Cursor<Vec<u8>>, String)> = if bundled {
            read_zip_from_disk(&bundle).map(|data| (data, bundle.display().to_string()))
        } else {
            read_zip_from_assets(&app)
                .map(|data| (data, format!("the APK's assets ({RESOURCE_ASSET})")))
        };
        if let Some((data, from)) = zip_source {
            log::info!(
                "Horizon: extracting resources ({from} -> {})",
                resources.display()
            );
            if let Err(e) = extract_zip(&data, &resources) {
                log::warn!("Horizon: could not extract {RESOURCE_ASSET}: {e}");
            }
        }

        horizon_client::android::entry(app, files);
    }

    /// The app's private files folder (`Context.getFilesDir`).
    fn files_dir(app: &AndroidApp) -> std::path::PathBuf {
        app.internal_data_path()
            .or_else(|| app.external_data_path())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }

    /// The bundle a player copied into `files/import/` with adb or a file
    /// manager.
    fn read_zip_from_disk(path: &std::path::Path) -> Option<std::io::Cursor<Vec<u8>>> {
        let data = std::fs::read(path).ok()?;
        Some(std::io::Cursor::new(data))
    }

    /// The bundle `tools/build_android.py --resources` packed into the APK.
    fn read_zip_from_assets(app: &AndroidApp) -> Option<std::io::Cursor<Vec<u8>>> {
        let manager = app.asset_manager();
        let name = std::ffi::CString::new(RESOURCE_ASSET).ok()?;
        let mut asset = manager.open(&name)?;
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut asset, &mut data).ok()?;
        Some(std::io::Cursor::new(data))
    }

    /// Writes every entry of the zip into `into`, creating folders as needed.
    fn extract_zip(data: &std::io::Cursor<Vec<u8>>, into: &std::path::Path) -> Result<(), String> {
        let mut archive = zip::ZipArchive::new(data.clone()).map_err(|e| e.to_string())?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
            let Some(name) = file.enclosed_name().map(|p| p.to_path_buf()) else {
                continue;
            };
            let out = into.join(name);
            if file.is_dir() {
                std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut bytes = Vec::with_capacity(file.size() as usize);
            std::io::Read::read_to_end(&mut file, &mut bytes).map_err(|e| e.to_string())?;
            std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
