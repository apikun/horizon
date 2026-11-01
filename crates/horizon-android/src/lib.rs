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
//! What the APK always carries is the placeholder texture pack
//! (`android/placeholder/8x8.zip`, built in as `placeholder-pack.zip`): it
//! is installed as the game folder's first resource pack, so a fresh
//! install draws with placeholder textures from the first start. The
//! game's data files (blocks, items, biomes — prepared from the player's
//! own Minecraft) are still the player's to supply; the missing-files
//! screen says how.
//!
//! On the desktop this crate is an empty stub (CI clippy's `--workspace`
//! still builds it): the Android module below is
//! `cfg(target_os = "android")`, and only the options.txt helper with its
//! tests compiles everywhere (so `cargo test` on a desktop covers it).
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
    /// The placeholder texture pack asset (`tools/build_android.py` copies
    /// `android/placeholder/8x8.zip` there), the file it becomes in the
    /// game folder's `resourcepacks/`, and its id in options.txt's
    /// `resourcePacks` list.
    const PLACEHOLDER_ASSET: &str = "placeholder-pack.zip";
    const PLACEHOLDER_PACK_FILE: &str = "8x8.zip";
    const PLACEHOLDER_PACK_ID: &str = "file/8x8.zip";

    #[unsafe(no_mangle)]
    fn android_main(app: AndroidApp) {
        horizon_threads::configure_allocator();
        std::hint::black_box(&MOD_API_LEVEL);

        // The game folder: the app's private files dir. Everything the
        // desktop run keeps in its working directory (saves, options.txt,
        // packs, logs) lives under it.
        let files = files_dir(&app);
        let resources = files.join(RESOURCES_DIR);
        install_placeholder_pack(&app, &files);
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
        read_asset(app, RESOURCE_ASSET).map(std::io::Cursor::new)
    }

    /// An APK asset's bytes (the asset manager opens it uncompressed).
    fn read_asset(app: &AndroidApp, name: &str) -> Option<Vec<u8>> {
        let manager = app.asset_manager();
        let name = std::ffi::CString::new(name).ok()?;
        let mut asset = manager.open(&name)?;
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut asset, &mut data).ok()?;
        Some(data)
    }

    /// Installs the placeholder texture pack as the game folder's first
    /// resource pack, so a fresh install draws with placeholder textures
    /// from the first start. Runs on every launch: it rewrites the pack
    /// only when the installed copy differs from the build's (an app
    /// update's newer pack replaces it, and a player who turned the pack
    /// off keeps it off), and enabling it in options.txt rides along with
    /// that rewrite.
    fn install_placeholder_pack(app: &AndroidApp, files: &std::path::Path) {
        let Some(data) = read_asset(app, PLACEHOLDER_ASSET) else {
            log::info!("Horizon: no {PLACEHOLDER_ASSET} in this APK: no placeholder textures");
            return;
        };
        let packs = files.join("game").join("resourcepacks");
        let pack = packs.join(PLACEHOLDER_PACK_FILE);
        let installed = std::fs::read(&pack)
            .map(|installed| installed == data)
            .unwrap_or(false);
        if installed {
            return;
        }
        let written = std::fs::create_dir_all(&packs).and_then(|()| std::fs::write(&pack, &data));
        if let Err(error) = written {
            log::warn!(
                "Horizon: could not install the placeholder pack {}: {error}",
                pack.display()
            );
            return;
        }
        log::info!(
            "Horizon: placeholder textures installed ({}, {} bytes)",
            pack.display(),
            data.len()
        );
        crate::options::enable_pack(&files.join("game").join("options.txt"), PLACEHOLDER_PACK_ID);
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

/// options.txt's `resourcePacks` list, edited in place. No Android API in
/// here, so the desktop `cargo test` runs its tests.
#[cfg(any(target_os = "android", test))]
pub(crate) mod options {
    /// Adds `id` to the `resourcePacks` list of an options.txt (creating the
    /// file when it is not there yet), unless the list already names it.
    /// The lists are Gson string arrays (`resourcePacks:["file/8x8.zip"]`);
    /// a line for the key is edited in place, everything else is kept.
    pub fn enable_pack(options: &std::path::Path, id: &str) {
        const KEY: &str = "resourcePacks:";
        let text = std::fs::read_to_string(options).unwrap_or_default();
        let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
        let item = format!("\"{id}\"");
        let mut found = false;
        for line in lines.iter_mut() {
            let Some(rest) = line.strip_prefix(KEY) else {
                continue;
            };
            found = true;
            if rest.contains(&item) {
                return;
            }
            let trimmed = rest.trim_end_matches(['\n', '\r']);
            let spaces: String =
                rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()].to_owned();
            let trailing: String = rest[trimmed.len()..].to_owned();
            // Inside the brackets: append as the last (top) item.
            let inner = trimmed
                .trim()
                .strip_prefix('[')
                .and_then(|inner| inner.strip_suffix(']'))
                .unwrap_or("");
            let list = match inner.trim() {
                "" => format!("[{item}]"),
                items => format!("[{},{}]", items.trim_end_matches(','), item),
            };
            *line = format!("{KEY}{spaces}{list}{trailing}");
            break;
        }
        let mut out: String = lines.into_iter().collect();
        if !found {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(KEY);
            out.push_str(&format!("[{item}]\n"));
        }
        if let Err(error) = std::fs::write(options, out) {
            log::warn!(
                "Horizon: could not enable the placeholder pack in {}: {error}",
                options.display()
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicU32, Ordering};

        fn written(before: Option<&str>, id: &str) -> String {
            static RUN: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "horizon-android-options-{}-{}-{}",
                std::process::id(),
                RUN.fetch_add(1, Ordering::Relaxed),
                id.replace('/', "_")
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("options.txt");
            if let Some(text) = before {
                std::fs::write(&file, text).unwrap();
            }
            enable_pack(&file, id);
            let out = std::fs::read_to_string(&file).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            out
        }

        #[test]
        fn a_missing_file_gets_just_the_list() {
            assert_eq!(
                written(None, "file/8x8.zip"),
                "resourcePacks:[\"file/8x8.zip\"]\n"
            );
        }

        #[test]
        fn a_file_without_the_key_keeps_its_lines_and_gains_the_list() {
            let before = "version:4620\nfov:90\n";
            assert_eq!(
                written(Some(before), "file/8x8.zip"),
                "{before}resourcePacks:[\"file/8x8.zip\"]\n".replace("{before}", before)
            );
        }

        #[test]
        fn an_existing_list_gains_the_pack_as_its_top_item() {
            assert_eq!(
                written(Some("resourcePacks:[\"file/a.zip\"]\n"), "file/8x8.zip"),
                "resourcePacks:[\"file/a.zip\",\"file/8x8.zip\"]\n"
            );
            assert_eq!(
                written(Some("resourcePacks:[]\n"), "file/8x8.zip"),
                "resourcePacks:[\"file/8x8.zip\"]\n"
            );
        }

        #[test]
        fn a_list_already_naming_the_pack_is_untouched() {
            let before = "resourcePacks:[\"file/8x8.zip\"]\n";
            assert_eq!(written(Some(before), "file/8x8.zip"), before);
        }

        #[test]
        fn the_incompatible_list_is_not_the_resource_list() {
            let before = "incompatibleResourcePacks:[\"file/8x8.zip\"]\n";
            assert_eq!(
                written(Some(before), "file/8x8.zip"),
                "{before}resourcePacks:[\"file/8x8.zip\"]\n".replace("{before}", before)
            );
        }

        #[test]
        fn a_file_without_a_final_newline_is_repaired_not_corrupted() {
            assert_eq!(
                written(Some("fov:90"), "file/8x8.zip"),
                "fov:90\nresourcePacks:[\"file/8x8.zip\"]\n"
            );
        }
    }
}
