//! Multiplayer section commands, they install only the official BalatroMultiplayer packages
//! from Thunderstore and only for casual play

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bmm_lib::database::Database;
use bmm_lib::local_mod_detection;
use serde::Serialize;

use crate::commands::install::{self as install_cmd, InstalledCatalogMod, PlanRun};
use crate::install_plan::{self, InstallPlan, InstallSource};
use crate::multiplayer::{
    self, ApiInstall, Detection, FoundMod, GameModeInfo, InstalledCopy, LaunchPlatform,
    ReadinessCheck, Removal, Role, Track, TrackInfo, TrackPlan,
};
use crate::state::AppState;
use crate::thunderstore::{self, TsPackage};
use crate::util::map_error;

const COPIED_DLLS_SETTING: &str = "multiplayer_copied_dlls";

#[derive(Debug, Clone, Serialize)]
pub struct MultiplayerStatus {
    pub namespace: String,
    /// BMM only installs for casual play, ranked needs the team's launcher
    pub casual_only: bool,
    pub bet_download_url: String,
    pub listing_available: bool,
    pub listing_error: Option<String>,
    pub platform: LaunchPlatform,
    pub tracks: Vec<TrackInfo>,
    pub active_track: Option<Track>,
    pub conflict: bool,
    /// A legacy install from outside Thunderstore, offered an upgrade
    pub legacy_manual: Option<InstalledCopy>,
    pub gamemodes: Vec<GameModeInfo>,
    pub readiness: Vec<ReadinessCheck>,
    /// DLLs BMM copied into a Proton save folder, removed with the API track
    pub copied_dlls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiplayerInstallResult {
    pub track: Track,
    pub installed: Vec<InstalledCatalogMod>,
    /// Row names, or folder names for untracked copies, that were removed
    pub removed: Vec<String>,
    pub plans: Vec<InstallPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiplayerUninstallResult {
    pub track: Track,
    pub removed: Vec<String>,
    pub removed_dlls: Vec<String>,
}

struct LocalState {
    packages: Vec<TsPackage>,
    listing_error: Option<String>,
    installed: Vec<bmm_lib::database::InstalledMod>,
    detection: Detection,
}

fn existing_mods_dirs() -> Vec<PathBuf> {
    install_cmd::installed_mods_dirs()
        .into_iter()
        .filter(|d| d.is_dir())
        .collect()
}

async fn scan_mods() -> Result<Vec<FoundMod>, String> {
    let dirs = existing_mods_dirs();
    tauri::async_runtime::spawn_blocking(move || multiplayer::scan_mods_dirs(&dirs))
        .await
        .map_err(|e| format!("Failed to scan the Mods folder: {e}"))
}

async fn local_state(db: &Mutex<Database>, refresh: bool) -> Result<LocalState, String> {
    let packages = if refresh {
        thunderstore::sync(false).await
    } else {
        thunderstore::cached_packages().await
    };
    let (packages, listing_error) = match packages {
        Ok(packages) => (packages, thunderstore::last_sync_error()),
        Err(e) => (Vec::new(), Some(e)),
    };
    let packages = crate::catalog::multiplayer_packages_in(&packages);
    let installed = {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.get_installed_mods())?
    };
    let found = scan_mods().await?;
    let detection = multiplayer::detect(&found, &installed, &packages);
    Ok(LocalState {
        packages,
        listing_error,
        installed,
        detection,
    })
}

pub(crate) fn copied_dlls(db: &Database) -> Vec<PathBuf> {
    db.get_setting(COPIED_DLLS_SETTING)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<Vec<PathBuf>>(&json).ok())
        .unwrap_or_default()
}

fn set_copied_dlls(db: &Database, paths: &[PathBuf]) {
    let result = if paths.is_empty() {
        db.remove_setting(COPIED_DLLS_SETTING)
    } else {
        match serde_json::to_string(paths) {
            Ok(json) => db.set_setting(COPIED_DLLS_SETTING, &json),
            Err(e) => {
                log::warn!("Failed to encode copied DLL list: {e}");
                return;
            }
        }
    };
    if let Err(e) = result {
        log::warn!("Failed to record copied Multiplayer DLLs: {e}");
    }
}

fn remove_tracked_dlls(db: &Mutex<Database>) -> Vec<String> {
    let db = db.lock().unwrap_or_else(|e| e.into_inner());
    let tracked = copied_dlls(&db);
    if tracked.is_empty() {
        return Vec::new();
    }
    let removed = multiplayer::remove_copied_dlls(&tracked);
    set_copied_dlls(&db, &[]);
    removed
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

#[tauri::command]
pub async fn multiplayer_status(
    state: tauri::State<'_, AppState>,
) -> Result<MultiplayerStatus, String> {
    let local = local_state(&state.db, false).await?;
    let (linux_prefix, copied) = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        (db.get_linux_prefix().ok().flatten(), copied_dlls(&db))
    };
    let platform = LaunchPlatform::current(linux_prefix.as_deref());
    let detection = &local.detection;
    let registry = detection
        .api_dir()
        .map(|dir| multiplayer::read_registry_modes(&dir))
        .unwrap_or_default();

    let check_steam = detection.has(Track::Api);
    let steam_running = if check_steam {
        tauri::async_runtime::spawn_blocking(bmm_lib::finder::is_steam_running)
            .await
            .unwrap_or(false)
    } else {
        false
    };
    let linux_libssl = (platform == LaunchPlatform::Linux)
        .then(|| {
            multiplayer::find_system_lib(multiplayer::LINUX_LIBSSL, &multiplayer::LINUX_LIB_DIRS)
        })
        .flatten();
    let mods_dir = local_mod_detection::resolve_mods_dir_path().ok();
    let (steamodded_installed, steamodded_version) =
        install_plan::installed_steamodded(&local.installed, mods_dir.as_deref());

    let readiness = multiplayer::readiness(&multiplayer::ReadinessInputs {
        platform,
        detection,
        packages: &local.packages,
        steam_running,
        linux_libssl,
        steamodded_installed,
        steamodded_version,
    });

    Ok(MultiplayerStatus {
        namespace: thunderstore::MULTIPLAYER_NAMESPACE.to_string(),
        casual_only: true,
        bet_download_url: multiplayer::BET_DOWNLOAD_URL.to_string(),
        listing_available: !local.packages.is_empty(),
        listing_error: local.listing_error.clone(),
        platform,
        tracks: vec![
            multiplayer::track_info(detection, &local.packages, Track::Legacy),
            multiplayer::track_info(detection, &local.packages, Track::Api),
        ],
        active_track: detection.active_track(),
        conflict: detection.conflict(),
        legacy_manual: detection.legacy_manual().cloned(),
        gamemodes: multiplayer::gamemode_infos(detection, &local.packages, &registry),
        readiness,
        copied_dlls: copied
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    })
}

/// What installing `track` with `gamemodes` would install and remove
#[tauri::command]
pub async fn multiplayer_plan(
    state: tauri::State<'_, AppState>,
    track: String,
    gamemodes: Option<Vec<String>>,
) -> Result<TrackPlan, String> {
    let track = Track::parse(&track)?;
    let local = local_state(&state.db, true).await?;
    multiplayer::plan_track(
        &local.detection,
        &local.packages,
        track,
        &gamemodes.unwrap_or_default(),
    )
}

/// Installs or updates a track and its game modes, switching tracks removes the other one
/// after the install succeeded, which needs `confirmRemoval`
#[tauri::command]
pub async fn multiplayer_install(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    track: String,
    gamemodes: Option<Vec<String>>,
    confirm_removal: Option<bool>,
    install_steamodded: Option<bool>,
) -> Result<MultiplayerInstallResult, String> {
    let track = Track::parse(&track)?;
    let local = local_state(&state.db, true).await?;
    let plan = multiplayer::plan_track(
        &local.detection,
        &local.packages,
        track,
        &gamemodes.unwrap_or_default(),
    )?;
    if plan.needs_confirmation && !confirm_removal.unwrap_or(false) {
        let names: Vec<String> = plan
            .remove
            .iter()
            .map(|r| r.tracked_name.clone().unwrap_or_else(|| r.folder.clone()))
            .collect();
        return Err(format!(
            "Installing this track removes {}, confirm to continue",
            names.join(", ")
        ));
    }
    execute(
        &state,
        &app_handle,
        plan,
        install_steamodded.unwrap_or(false),
    )
    .await
}

/// Replaces a manual legacy install (the old GitHub release) with the official package,
/// the old folder is removed only after the install succeeded and its row keeps its name
#[tauri::command]
pub async fn multiplayer_migrate_legacy(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    install_steamodded: Option<bool>,
) -> Result<MultiplayerInstallResult, String> {
    let local = local_state(&state.db, true).await?;
    let plan = multiplayer::plan_legacy_migration(&local.detection, &local.packages)?;
    execute(
        &state,
        &app_handle,
        plan,
        install_steamodded.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn multiplayer_uninstall(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    track: String,
) -> Result<MultiplayerUninstallResult, String> {
    let track = Track::parse(&track)?;
    let local = local_state(&state.db, false).await?;
    let mut removed = Vec::new();
    let mut result = Ok(());
    for target in multiplayer::uninstall_targets(&local.detection, track) {
        match remove_copy(&state.db, &target).await {
            Ok(label) => removed.push(label),
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    if result.is_ok() {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        for name in multiplayer::stale_rows_of(&local.detection, track) {
            if let Err(e) = db.remove_installed_mod(&name) {
                log::warn!("Failed to remove the stale {name} row: {e}");
            } else {
                removed.push(name);
            }
        }
    }
    let removed_dlls = if track == Track::Api && result.is_ok() {
        remove_tracked_dlls(&state.db)
    } else {
        Vec::new()
    };

    if !removed.is_empty() {
        install_cmd::sync_compat_helper_after_mod_change(&state).await;
        install_cmd::refresh_mod_detection_cache();
        install_cmd::emit_installed_mods_changed(&app_handle, Vec::new(), removed.clone());
    }
    result?;
    Ok(MultiplayerUninstallResult {
        track,
        removed,
        removed_dlls,
    })
}

/// Deletes a copy's folder and its row, returns the name shown to the user
async fn remove_copy(db: &Mutex<Database>, removal: &Removal) -> Result<String, String> {
    let path = PathBuf::from(&removal.path);
    if path.exists() {
        map_error(bmm_lib::installer::uninstall_mod(path).await)?;
    }
    if let Some(name) = &removal.tracked_name {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.remove_installed_mod_by_name_or_path(name, &removal.path))?;
    }
    Ok(removal
        .tracked_name
        .clone()
        .unwrap_or_else(|| removal.folder.clone()))
}

/// The row of a manual copy that the official package takes over, unless another row
/// already tracks that package
fn adopted_row(
    plan: &TrackPlan,
    full_name: &str,
    installed: &[bmm_lib::database::InstalledMod],
) -> Option<(String, String)> {
    let already_tracked = installed.iter().any(|r| {
        r.source_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(full_name))
    });
    if already_tracked {
        return None;
    }
    plan.replace
        .iter()
        .filter(|r| {
            r.replaced_by
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(full_name))
        })
        .filter_map(|r| r.tracked_name.as_deref())
        .find_map(|name| installed.iter().find(|row| row.name == name))
        .map(|row| (row.name.clone(), row.source.clone()))
}

async fn execute(
    state: &tauri::State<'_, AppState>,
    app_handle: &tauri::AppHandle,
    plan: TrackPlan,
    install_steamodded: bool,
) -> Result<MultiplayerInstallResult, String> {
    let mods_dirs = install_cmd::installed_mods_dirs();
    let mut run = PlanRun {
        installed: Vec::new(),
        warnings: plan.warnings.clone(),
        removed: 0,
    };
    let mut plans = Vec::new();
    let mut removed = Vec::new();
    let result = execute_steps(
        state,
        &plan,
        install_steamodded,
        &mods_dirs,
        &mut run,
        &mut plans,
        &mut removed,
    )
    .await;

    if !run.installed.is_empty() || run.removed > 0 || !removed.is_empty() {
        install_cmd::sync_compat_helper_after_mod_change(state).await;
        install_cmd::refresh_mod_detection_cache();
        install_cmd::emit_installed_mods_changed(
            app_handle,
            run.installed.iter().map(|m| m.name.clone()).collect(),
            removed.clone(),
        );
    }
    for warning in &run.warnings {
        log::warn!("Multiplayer install: {warning}");
    }
    result?;
    Ok(MultiplayerInstallResult {
        track: plan.track,
        installed: run.installed,
        removed,
        plans,
        warnings: run.warnings,
    })
}

async fn execute_steps(
    state: &tauri::State<'_, AppState>,
    plan: &TrackPlan,
    install_steamodded: bool,
    mods_dirs: &[PathBuf],
    run: &mut PlanRun,
    plans: &mut Vec<InstallPlan>,
    removed: &mut Vec<String>,
) -> Result<(), String> {
    let mut adopted: Vec<String> = Vec::new();
    for full_name in &plan.packages {
        // Planned one by one so each package sees what the previous ones installed
        let inputs = install_cmd::load_plan_inputs(state).await?;
        let mut package_plan = install_plan::plan_install(
            &inputs.context(),
            &format!("{}{full_name}", crate::catalog::TS_ID_PREFIX),
            None,
            Some(InstallSource::Thunderstore),
        )?;
        if let Some((name, source)) = adopted_row(plan, full_name, &inputs.installed)
            && let Some(root) = package_plan.steps.iter_mut().find(|s| s.is_root)
            && root.tracked_name.is_none()
        {
            root.tracked_name = Some(name.clone());
            root.tracked_source = Some(source);
            adopted.push(name);
        }
        run.warnings.extend(package_plan.warnings.iter().cloned());
        install_cmd::run_plan(&state.db, &package_plan, install_steamodded, mods_dirs, run).await?;
        plans.push(package_plan);
    }

    for old in &plan.replace {
        let old_path = Path::new(&old.path);
        if !old_path.exists() {
            continue;
        }
        let new_path = old.replaced_by.as_deref().and_then(|package| {
            run.installed
                .iter()
                .find(|m| {
                    m.source_id
                        .as_deref()
                        .is_some_and(|id| id.eq_ignore_ascii_case(package))
                })
                .map(|m| m.path.clone())
        });
        let adopted_here = old
            .tracked_name
            .as_ref()
            .is_some_and(|n| adopted.contains(n));
        match new_path {
            Some(new_path) if !adopted_here => {
                replace_copy(&state.db, old, &new_path, mods_dirs).await?;
                if !old_path.exists() {
                    removed.push(
                        old.tracked_name
                            .clone()
                            .unwrap_or_else(|| old.folder.clone()),
                    );
                }
            }
            _ => run.warnings.push(format!(
                "The old copy at {} is still there, remove it so it does not load twice",
                old.path
            )),
        }
    }

    let mut api_removed = false;
    for target in &plan.remove {
        removed.push(remove_copy(&state.db, target).await?);
        api_removed |= target.role == Role::Api;
    }
    if api_removed {
        remove_tracked_dlls(&state.db);
    }
    Ok(())
}

/// Removes a manual copy the official package replaced, a disabled copy keeps the new
/// install disabled
async fn replace_copy(
    db: &Mutex<Database>,
    old: &Removal,
    new_path: &Path,
    mods_dirs: &[PathBuf],
) -> Result<(), String> {
    let protected: Vec<PathBuf> = {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.get_installed_mods())?
            .into_iter()
            .filter(|m| Some(&m.name) != old.tracked_name.as_ref())
            .filter(|m| !m.path.trim().is_empty() && m.path != old.path)
            .map(|m| PathBuf::from(m.path))
            .collect()
    };
    let name = old
        .tracked_name
        .clone()
        .unwrap_or_else(|| old.folder.clone());
    let old_path = PathBuf::from(&old.path);
    let new_path = new_path.to_path_buf();
    let mods_dirs = mods_dirs.to_vec();
    let cleanup_name = name.clone();
    let cleanup_old = old_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        bmm_lib::installer::cleanup_replaced_install(
            &cleanup_name,
            Some(&cleanup_old),
            &new_path,
            &mods_dirs,
            &protected,
        )
    })
    .await
    .map_err(|e| format!("Cleanup after replacing {name} failed: {e}"))?;
    if !old_path.exists()
        && let Some(row) = &old.tracked_name
    {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        map_error(db.remove_installed_mod_by_name_or_path(row, &old.path))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------

/// The enabled MultiplayerAPI in the Mods folder the game loads
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub(crate) fn launch_api() -> Option<ApiInstall> {
    let mods_dir = local_mod_detection::resolve_mods_dir_path().ok()?;
    multiplayer::installed_api(&multiplayer::scan_mods_dir(&mods_dir))
}

/// Removes BET_ variables the game would inherit from BMM
pub(crate) fn scrub_bet_env(cmd: &mut std::process::Command) {
    for key in multiplayer::inherited_bet_keys() {
        log::warn!("Not passing {key:?} to Balatro, BMM never sets BET variables");
        cmd.env_remove(key);
    }
}

/// Puts the API's networking folder first on PATH so LuaJIT finds the bundled DLLs,
/// untested on real Windows
#[cfg(target_os = "windows")]
pub(crate) fn apply_windows_launch(cmd: &mut std::process::Command) {
    let Some(api) = launch_api() else {
        return;
    };
    let inherited = std::env::var("PATH").ok();
    let launch = multiplayer::launch_env(
        multiplayer::LaunchKind::Windows,
        Some(&api),
        inherited.as_deref(),
    );
    for (key, value) in launch.env {
        log::info!("Multiplayer: adding {} to {key}", api.networking.display());
        cmd.env(key, value);
    }
}

/// Adds WINEPATH for the API, or copies its DLLs into the Proton save folder when Steam
/// starts the game and our environment is lost, untested on real hardware
#[cfg(target_os = "linux")]
pub(crate) fn apply_wine_launch(
    db: &Mutex<Database>,
    game_path: &Path,
    env_passable: bool,
    envs: &mut Vec<(String, String)>,
) {
    let Some(api) = launch_api() else {
        return;
    };
    let existing = envs
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case("WINEPATH"));
    let inherited = existing
        .map(|i| envs[i].1.clone())
        .or_else(|| std::env::var("WINEPATH").ok());
    let launch = multiplayer::launch_env(
        multiplayer::LaunchKind::Wine { env_passable },
        Some(&api),
        inherited.as_deref(),
    );
    for (key, value) in launch.env {
        log::info!("Multiplayer: {key}={value}");
        match envs.iter().position(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            Some(i) => envs[i].1 = value,
            None => envs.push((key, value)),
        }
    }
    if !launch.copy_dlls_to_save_dir {
        return;
    }
    let Some(save_dir) = multiplayer::proton_save_dir(game_path) else {
        log::warn!(
            "Multiplayer: cannot locate the Proton save folder for {}, the API may not find OpenSSL",
            game_path.display()
        );
        return;
    };
    let db = db.lock().unwrap_or_else(|e| e.into_inner());
    let mut tracked = copied_dlls(&db);
    match multiplayer::copy_dlls_to_save_dir(&api.networking, &save_dir, &tracked) {
        Ok(owned) => {
            for path in owned {
                if !tracked.contains(&path) {
                    tracked.push(path);
                }
            }
            set_copied_dlls(&db, &tracked);
            log::info!(
                "Multiplayer: copied the OpenSSL DLLs to {}",
                save_dir.display()
            );
        }
        Err(e) => log::warn!("Multiplayer: failed to copy the OpenSSL DLLs: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplayer::{InstalledCopy, Role};
    use bmm_lib::database::{InstalledMod, SOURCE_INDEX, SOURCE_THUNDERSTORE};

    fn row(name: &str, source: &str, source_id: Option<&str>) -> InstalledMod {
        InstalledMod {
            name: name.into(),
            path: format!("/mods/{name}"),
            dependencies: Vec::new(),
            current_version: None,
            orphaned: true,
            source: source.into(),
            source_id: source_id.map(str::to_string),
            installed_at: None,
        }
    }

    fn legacy_plan() -> TrackPlan {
        let packages = crate::thunderstore::tests::fixture_packages();
        let detection = Detection {
            copies: vec![InstalledCopy {
                role: Role::Legacy,
                path: "/mods/Multiplayer".into(),
                folder: "Multiplayer".into(),
                mod_id: "Multiplayer".into(),
                mod_version: Some("0.5.4".into()),
                package: None,
                package_version: None,
                tracked_name: Some("Multiplayer".into()),
                tracked_source: Some(SOURCE_INDEX.into()),
                tracked_version: Some("v0.5.4".into()),
                official: false,
                disabled: false,
            }],
            stale_rows: Vec::new(),
        };
        multiplayer::plan_legacy_migration(&detection, &packages).unwrap()
    }

    #[test]
    fn legacy_row_is_adopted_unless_the_package_is_tracked() {
        let plan = legacy_plan();
        let installed = [row("Multiplayer", SOURCE_INDEX, None)];
        assert_eq!(
            adopted_row(&plan, multiplayer::LEGACY_PACKAGE, &installed),
            Some(("Multiplayer".to_string(), SOURCE_INDEX.to_string()))
        );
        assert_eq!(
            adopted_row(&plan, multiplayer::API_PACKAGE, &installed),
            None
        );
        let installed = [
            row("Multiplayer", SOURCE_INDEX, None),
            row(
                "Multiplayer (BalatroMultiplayer)",
                SOURCE_THUNDERSTORE,
                Some(multiplayer::LEGACY_PACKAGE),
            ),
        ];
        assert_eq!(
            adopted_row(&plan, multiplayer::LEGACY_PACKAGE, &installed),
            None
        );
    }

    #[test]
    fn copied_dll_list_roundtrips() {
        let td = tempfile::tempdir().unwrap();
        let db = Database::open_at(&td.path().join("bmm_storage.db")).unwrap();
        assert!(copied_dlls(&db).is_empty());
        let paths = vec![td.path().join("save/libssl-3-x64.dll")];
        set_copied_dlls(&db, &paths);
        assert_eq!(copied_dlls(&db), paths);
        set_copied_dlls(&db, &[]);
        assert!(copied_dlls(&db).is_empty());
    }
}
