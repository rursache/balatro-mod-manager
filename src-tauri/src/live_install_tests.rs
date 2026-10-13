//! End to end installs against the live Thunderstore, into a temp Mods folder and a temp
//! database, never the user's game folder:
//! `cargo test live_install -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bmm_lib::database::{Database, SOURCE_THUNDERSTORE};

use crate::commands::install::{PlanRun, run_plan};
use crate::install_plan::{self, InstallPlan, InstallSource, PlanContext};
use crate::multiplayer::{self, CheckStatus, LaunchPlatform, Track};
use crate::thunderstore::{self, TsPackage};
use crate::{catalog, models::ArchiveModItem};

fn plan(
    catalog: &[ArchiveModItem],
    packages: &[TsPackage],
    db: &Mutex<Database>,
    id: &str,
) -> InstallPlan {
    let installed = db.lock().unwrap().get_installed_mods().unwrap();
    let context = PlanContext {
        catalog,
        packages,
        installed: &installed,
        lovely_version: None,
        steamodded_installed: false,
        steamodded_version: None,
        talisman_installed: false,
    };
    install_plan::plan_install(&context, id, None, Some(InstallSource::Thunderstore))
        .unwrap_or_else(|e| panic!("plan {id}: {e}"))
}

async fn install(plan: &InstallPlan, db: &Mutex<Database>, mods: &Path) -> PlanRun {
    let mut run = PlanRun {
        installed: Vec::new(),
        warnings: plan.warnings.clone(),
        removed: 0,
        mods_dir: Some(mods.to_path_buf()),
    };
    // Steamodded stays a version check, nothing outside the temp Mods folder is touched
    run_plan(db, plan, false, &[mods.to_path_buf()], &mut run)
        .await
        .unwrap_or_else(|e| panic!("install {}: {e}", plan.mod_id));
    run
}

/// Lovely reads `<mod>/lovely.toml` and `<mod>/lovely/*.toml`, nested payloads must be hoisted
fn lovely_patches(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "toml") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel == "lovely.toml" || rel.contains("lovely") {
                    found.push(rel);
                }
            }
        }
    }
    found.sort();
    found
}

fn assert_patch_depth(root: &Path) -> Vec<String> {
    let patches = lovely_patches(root);
    for rel in &patches {
        assert!(
            rel == "lovely.toml" || rel.starts_with("lovely/"),
            "{} has a Lovely patch Lovely will not load: {rel}",
            root.display()
        );
    }
    patches
}

async fn uninstall(db: &Mutex<Database>, mods: &Path, name: &str, path: &Path) {
    if path.exists() {
        bmm_lib::installer::uninstall_mod_from(path.to_path_buf(), &[mods.to_path_buf()])
            .await
            .unwrap_or_else(|e| panic!("uninstall {name}: {e}"));
    }
    db.lock()
        .unwrap()
        .remove_installed_mod_by_name_or_path(name, &path.to_string_lossy())
        .unwrap();
}

#[test]
#[ignore = "network: installs live Thunderstore packages into a temp Mods folder"]
fn live_install_joker_display_and_multiplayer_api() {
    tauri::async_runtime::block_on(async {
        let mut cache = thunderstore::TsCache::empty();
        thunderstore::refresh(&mut cache, &thunderstore::HttpFetcher, 0)
            .await
            .expect("listing");
        let packages = cache.packages;
        let catalog = catalog::merge(Vec::new(), &packages);

        let td = tempfile::tempdir().unwrap();
        let mods = td.path().join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        let db = Mutex::new(Database::open_at(&td.path().join("bmm_storage.db")).unwrap());

        // JokerDisplay from the general catalog
        let joker_plan = plan(&catalog, &packages, &db, "ts:nh6574-JokerDisplay");
        let run = install(&joker_plan, &db, &mods).await;
        println!(
            "JokerDisplay: installed {:?}, warnings {:?}",
            run.installed
                .iter()
                .map(|m| format!("{}@{}", m.name, m.version))
                .collect::<Vec<_>>(),
            run.warnings
        );
        let joker_dir = mods.join("nh6574-JokerDisplay");
        assert!(joker_dir.join("manifest.json").is_file());
        assert!(
            std::fs::read_dir(&joker_dir)
                .unwrap()
                .flatten()
                .any(|e| e.path().extension().is_some_and(|x| x == "lua")),
            "the mod's lua files sit at the folder root"
        );
        println!("JokerDisplay patches: {:?}", assert_patch_depth(&joker_dir));
        let row = db
            .lock()
            .unwrap()
            .get_installed_by_source_id("nh6574-JokerDisplay")
            .unwrap()
            .expect("tracked");
        assert_eq!(row.source, SOURCE_THUNDERSTORE);
        assert_eq!(Path::new(&row.path), joker_dir);

        // The API track, as the Multiplayer section installs it
        let found = multiplayer::scan_mods_dir(&mods);
        let detection = multiplayer::detect(&found, &[], &packages);
        let track = multiplayer::plan_track(&detection, &packages, Track::Api, &[]).unwrap();
        assert_eq!(track.packages, [multiplayer::API_PACKAGE]);
        assert!(!track.needs_confirmation);
        let api_plan = plan(
            &catalog,
            &packages,
            &db,
            &format!("{}{}", catalog::TS_ID_PREFIX, multiplayer::API_PACKAGE),
        );
        assert!(
            api_plan.steps.iter().all(|s| s
                .full_name
                .as_deref()
                .is_some_and(|n| n.starts_with(thunderstore::MULTIPLAYER_NAMESPACE))),
            "only official BalatroMultiplayer packages"
        );
        for step in &api_plan.steps {
            let (ns, name) =
                thunderstore::split_full_name(step.full_name.as_deref().unwrap()).unwrap();
            bmm_lib::installer::validate_thunderstore_download_url(&step.download_url, ns, name)
                .expect("official Thunderstore download");
        }
        install(&api_plan, &db, &mods).await;

        let api_dir = mods.join(multiplayer::API_PACKAGE);
        let patches = assert_patch_depth(&api_dir);
        println!("MultiplayerAPI patches: {patches:?}");
        assert!(!patches.is_empty(), "the API ships Lovely patches");
        let networking = api_dir.join("networking");
        for lib in multiplayer::MACOS_LIBS {
            assert!(networking.join(lib).is_file(), "{lib} is bundled");
        }
        #[cfg(target_os = "macos")]
        for lib in multiplayer::MACOS_LIBS {
            let quarantine = std::process::Command::new("xattr")
                .args(["-p", "com.apple.quarantine"])
                .arg(networking.join(lib))
                .output()
                .expect("xattr");
            assert!(!quarantine.status.success(), "{lib} is not quarantined");
        }

        let installed = db.lock().unwrap().get_installed_mods().unwrap();
        let detection =
            multiplayer::detect(&multiplayer::scan_mods_dir(&mods), &installed, &packages);
        assert_eq!(detection.active_track(), Some(Track::Api));
        let api = detection.of(multiplayer::Role::Api).next().unwrap();
        assert!(api.official && api.tracked_name.is_some());
        let readiness = multiplayer::readiness(&multiplayer::ReadinessInputs {
            platform: LaunchPlatform::Macos,
            detection: &detection,
            packages: &packages,
            steam_running: true,
            linux_libssl: None,
            steamodded_installed: false,
            steamodded_version: None,
        });
        let libs = readiness.iter().find(|c| c.id == "native_libs").unwrap();
        assert_eq!(libs.status, CheckStatus::Ok, "{libs:?}");

        // Uninstall the track the way the Multiplayer view does, then JokerDisplay
        let preview = multiplayer::uninstall_preview(&detection, &packages, Track::Api);
        assert!(preview.keep.is_empty());
        for target in &preview.remove {
            let name = target.tracked_name.clone().unwrap_or(target.folder.clone());
            uninstall(&db, &mods, &name, Path::new(&target.path)).await;
        }
        let rows = db.lock().unwrap().get_installed_mods().unwrap();
        for row in rows {
            uninstall(&db, &mods, &row.name, &PathBuf::from(&row.path)).await;
        }

        assert!(db.lock().unwrap().get_installed_mods().unwrap().is_empty());
        let left: Vec<_> = std::fs::read_dir(&mods)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(left.is_empty(), "Mods is empty again: {left:?}");
    });
}
