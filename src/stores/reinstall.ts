import { get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { addMessage } from "$lib/stores";
import {
  installationStatus,
  loadingStates2,
  modEnabledStore,
  showWarningPopup,
  updateAvailableStore,
  type InstalledMod,
  type Mod,
} from "./modStore";
import { fetchCachedMods, forceRefreshCache } from "./modCache";
import type { CatalogInstallResult } from "./catalogInstall";
import { catalogInstallId } from "../utils/catalogItems";
import {
  findInstalledRow,
  indexInstalledRows,
} from "../utils/installedPlaceholders";
import { reinstallTarget, type ReinstallTarget } from "../utils/reinstall";

export interface ReinstallOptions {
  /** Steamodded / Talisman tag to install, defaults to the installed one */
  version?: string | null;
  /** Skip the confirmation, for callers that already asked */
  confirm?: boolean;
}

export interface ReinstallResult {
  name: string;
  version: string | null;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function folderOf(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? "";
}

async function readEnabled(name: string): Promise<boolean> {
  try {
    return await invoke<boolean>("is_mod_enabled", { modName: name });
  } catch {
    return true;
  }
}

export async function installedRowFor(mod: Mod): Promise<InstalledMod | null> {
  const rows = await fetchCachedMods();
  return findInstalledRow(mod, indexInstalledRows(rows));
}

/** Asks before replacing the files, resolves false when cancelled */
export function confirmReinstall(
  mod: Mod,
  version?: string | null,
): Promise<boolean> {
  const label = version ? `${mod.title} ${version}` : mod.title;
  return new Promise((resolve) => {
    showWarningPopup.set({
      visible: true,
      message: `Reinstall ${label}? Its files are downloaded again and replace the installed copy, your mod settings stay`,
      onConfirm: () => resolve(true),
      onCancel: () => resolve(false),
    });
  });
}

async function runReinstall(
  mod: Mod,
  target: ReinstallTarget,
): Promise<ReinstallResult> {
  const { row } = target;
  switch (target.kind) {
    case "thunderstore": {
      const result = await invoke<CatalogInstallResult>("install_catalog_mod", {
        id: catalogInstallId(mod),
        version: target.version,
        source: "thunderstore",
        installSteamodded: false,
      });
      for (const warning of result.warnings) addMessage(warning, "warning");
      const sourceId = row.source_id?.toLowerCase();
      const root =
        result.installed.find(
          (r) => sourceId && r.source_id?.toLowerCase() === sourceId,
        ) ?? result.installed.find((r) => r.name === row.name);
      if (!root) throw new Error("the package was not installed");
      return { name: root.name, version: root.version || target.version };
    }
    case "index": {
      const path = await invoke<string>("install_mod", {
        url: target.url,
        folderName:
          folderOf(row.path) || mod.folderName || mod.title.replace(/\s+/g, ""),
      });
      await invoke("add_installed_mod", {
        name: row.name,
        path,
        dependencies: row.dependencies ?? [],
        currentVersion: target.version ?? "",
      });
      return { name: row.name, version: target.version };
    }
    case "steamodded":
    case "talisman": {
      const command =
        target.kind === "steamodded"
          ? "install_steamodded_version"
          : "install_talisman_version";
      const path = await invoke<string>(command, { version: target.version });
      const exists = await invoke<boolean>("verify_path_exists", { path });
      if (!exists) throw new Error("files not found at destination");
      await invoke("add_installed_mod", {
        name: row.name,
        path,
        dependencies: row.dependencies ?? [],
        currentVersion: target.version ?? "",
      });
      return { name: row.name, version: target.version };
    }
  }
}

/**
 * Reinstalls the installed version of a mod from the source it was installed from,
 * keeps its enabled state and resolves the recorded name and version, null when nothing ran
 */
export async function reinstallMod(
  mod: Mod,
  options: ReinstallOptions = {},
): Promise<ReinstallResult | null> {
  if (get(loadingStates2)[mod.title]) return null;

  let row: InstalledMod | null;
  try {
    row = await installedRowFor(mod);
  } catch (error) {
    addMessage(
      `Failed to reinstall ${mod.title}: ${errorText(error)}`,
      "error",
    );
    return null;
  }
  const target = reinstallTarget(mod, row, options.version);
  if (!target) {
    addMessage(`${mod.title} cannot be reinstalled from here`, "warning");
    return null;
  }
  if (options.confirm !== false) {
    const confirmed = await confirmReinstall(mod, target.version);
    if (!confirmed) return null;
  }
  if (get(loadingStates2)[mod.title]) return null;

  loadingStates2.update((s) => ({ ...s, [mod.title]: true }));
  try {
    const wasEnabled = await readEnabled(target.row.name);
    const result = await runReinstall(mod, target);
    if (!wasEnabled && (await readEnabled(result.name))) {
      await invoke("toggle_mod_enabled", {
        modName: result.name,
        enabled: false,
      });
    }
    modEnabledStore.update((s) => ({ ...s, [mod.title]: wasEnabled }));
    installationStatus.update((s) => ({ ...s, [mod.title]: true }));
    if (target.kind === "index") {
      updateAvailableStore.update((s) => ({ ...s, [mod.title]: false }));
    }
    addMessage(
      `Reinstalled ${mod.title}${result.version ? ` ${result.version}` : ""}`,
      "success",
    );
    return result;
  } catch (error) {
    console.error(`Failed to reinstall ${mod.title}:`, error);
    addMessage(
      `Failed to reinstall ${mod.title}: ${errorText(error)}`,
      "error",
    );
    return null;
  } finally {
    loadingStates2.update((s) => ({ ...s, [mod.title]: false }));
    void forceRefreshCache().catch(() => {});
  }
}
