import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { addMessage } from "$lib/stores";
import {
  installationStatus,
  loadingStates2,
  updateAvailableStore,
  type InstallSourceName,
  type Mod,
} from "./modStore";
import { forceRefreshCache } from "./modCache";
import { catalogInstallId } from "../utils/catalogItems";

export interface PlanStep {
  source: InstallSourceName;
  catalog_id: string;
  title: string;
  full_name: string | null;
  version: string;
  min_version: string | null;
  download_url: string;
  folder_name: string;
  requires: string[];
  requires_steamodded: boolean;
  requires_talisman: boolean;
  deprecated: boolean;
  tracked_name: string | null;
  tracked_source: string | null;
  installed_version: string | null;
  is_root: boolean;
}

export interface SatisfiedDependency {
  full_name: string;
  title: string;
  tracked_name: string;
  installed_version: string | null;
  min_version: string;
}

export interface InstallPlan {
  mod_id: string;
  mod_title: string;
  source: InstallSourceName;
  version: string;
  steps: PlanStep[];
  satisfied: SatisfiedDependency[];
  lovely: {
    min_version: string;
    installed_version: string | null;
    satisfied: boolean;
  } | null;
  steamodded: {
    min_version: string | null;
    min_tag: string | null;
    installed: boolean;
    installed_version: string | null;
    satisfied: boolean;
    legacy_api: boolean;
  } | null;
  talisman: { installed: boolean } | null;
  missing: string[];
  warnings: string[];
}

export interface InstalledCatalogMod {
  name: string;
  path: string;
  version: string;
  source: InstallSourceName;
  source_id: string | null;
}

export interface CatalogInstallResult {
  plan: InstallPlan;
  installed: InstalledCatalogMod[];
  warnings: string[];
}

export interface CatalogInstallOptions {
  source?: InstallSourceName | null;
  version?: string | null;
  /** False installs without asking, missing Steamodded is then installed too */
  prompt?: boolean;
}

export interface InstallPromptDecision {
  confirmed: boolean;
  installSteamodded: boolean;
}

export interface InstallPlanPromptState {
  open: boolean;
  plan: InstallPlan | null;
  resolve: ((decision: InstallPromptDecision) => void) | null;
}

export const installPlanPromptStore = writable<InstallPlanPromptState>({
  open: false,
  plan: null,
  resolve: null,
});

export function closeInstallPlanPrompt(decision: InstallPromptDecision) {
  const state = get(installPlanPromptStore);
  installPlanPromptStore.set({ open: false, plan: null, resolve: null });
  state.resolve?.(decision);
}

function askInstallPlan(plan: InstallPlan): Promise<InstallPromptDecision> {
  // A prompt still open belongs to an earlier click, treat it as cancelled
  closeInstallPlanPrompt({ confirmed: false, installSteamodded: false });
  return new Promise((resolve) => {
    installPlanPromptStore.set({ open: true, plan, resolve });
  });
}

/** Dependencies the plan installs besides the mod itself */
export function dependencySteps(plan: InstallPlan): PlanStep[] {
  return plan.steps.filter((s) => !s.is_root);
}

/** Whether the user should see the plan before anything is downloaded */
export function planNeedsPrompt(plan: InstallPlan): boolean {
  return (
    dependencySteps(plan).length > 0 ||
    plan.missing.length > 0 ||
    plan.warnings.length > 0 ||
    plan.lovely?.satisfied === false ||
    plan.steamodded?.satisfied === false ||
    plan.steps.some((s) => s.deprecated)
  );
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function setLoading(title: string, value: boolean) {
  loadingStates2.update((s) => ({ ...s, [title]: value }));
}

export function planCatalogInstall(
  mod: Mod,
  options: Pick<CatalogInstallOptions, "source" | "version"> = {},
): Promise<InstallPlan> {
  const id = catalogInstallId(mod);
  if (!id) return Promise.reject(new Error(`${mod.title} has no catalog id`));
  return invoke<InstallPlan>("plan_catalog_install", {
    id,
    version: options.version,
    source: options.source,
  });
}

/**
 * Installs a catalog entry through the backend planner, Thunderstore dependencies
 * included. The plan is shown first whenever it pulls in more than the mod itself.
 * Resolves true when something was installed
 */
export async function installCatalogEntry(
  mod: Mod,
  options: CatalogInstallOptions = {},
): Promise<boolean> {
  const id = catalogInstallId(mod);
  if (!id) return false;
  if (get(loadingStates2)[mod.title]) return false;
  setLoading(mod.title, true);
  try {
    const plan = await planCatalogInstall(mod, options);
    let installSteamodded = plan.steamodded?.satisfied === false;
    if (options.prompt !== false && planNeedsPrompt(plan)) {
      setLoading(mod.title, false);
      const decision = await askInstallPlan(plan);
      if (!decision.confirmed) return false;
      installSteamodded = decision.installSteamodded;
      setLoading(mod.title, true);
    }

    const result = await invoke<CatalogInstallResult>("install_catalog_mod", {
      id,
      version: options.version,
      source: options.source,
      installSteamodded,
    });
    for (const warning of result.warnings) addMessage(warning, "warning");
    if (plan.lovely && !plan.lovely.satisfied) {
      addMessage(
        `${mod.title} needs Lovely ${plan.lovely.min_version} or newer`,
        "warning",
      );
    }
    installationStatus.update((s) => ({ ...s, [mod.title]: true }));
    updateAvailableStore.update((s) => {
      const next = { ...s, [mod.title]: false };
      for (const row of result.installed) next[row.name] = false;
      return next;
    });
    return result.installed.length > 0;
  } catch (error) {
    console.error(`Failed to install ${mod.title}:`, error);
    addMessage(`Installation failed: ${errorText(error)}`, "error");
    return false;
  } finally {
    setLoading(mod.title, false);
    // A failed run may still have installed some dependencies
    void forceRefreshCache().catch(() => {});
  }
}
