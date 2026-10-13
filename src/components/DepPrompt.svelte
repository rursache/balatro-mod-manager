<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { get } from "svelte/store";
  import {
    depPromptStore,
    closeDepPrompt,
    setModInCollection,
  } from "../stores/collections";
  import {
    modsStore,
    installationStatus,
    loadingStates2,
  } from "../stores/modStore";
  import { invoke } from "@tauri-apps/api/core";
  import { addMessage } from "$lib/stores";
  import type { Mod } from "../stores/modStore";
  import { modal } from "../utils/modalStack";
  import {
    closeInstallPlanPrompt,
    dependencySteps,
    installCatalogEntry,
    installPlanPromptStore,
  } from "../stores/catalogInstall";
  import {
    settingsInstallSource,
    usesCatalogInstall,
  } from "../utils/catalogItems";
  import { thunderstoreEnabled } from "../stores/catalogFilters";

  let installSteamodded = $state(true);
  let lovelyUpdating = $state(false);
  let lovelyUpdated = $state(false);
  let planSteps = $derived(
    $installPlanPromptStore.plan
      ? dependencySteps($installPlanPromptStore.plan)
      : [],
  );
  let rootStep = $derived(
    $installPlanPromptStore.plan?.steps.find((s) => s.is_root) ?? null,
  );

  $effect(() => {
    if ($installPlanPromptStore.open) {
      installSteamodded = true;
      lovelyUpdated = false;
    }
  });

  function sourceLine(source: string, version?: string | null): string {
    const from = source === "thunderstore" ? "Thunderstore" : "the mod index";
    return version ? `From ${from}, version ${version}` : `From ${from}`;
  }

  function cancelPlan() {
    closeInstallPlanPrompt({ confirmed: false, installSteamodded: false });
  }

  function confirmPlan() {
    const plan = $installPlanPromptStore.plan;
    closeInstallPlanPrompt({
      confirmed: true,
      installSteamodded:
        installSteamodded && plan?.steamodded?.satisfied === false,
    });
  }

  async function updateLovely() {
    if (lovelyUpdating) return;
    lovelyUpdating = true;
    try {
      const version = await invoke<string>("update_lovely_to_latest");
      lovelyUpdated = true;
      addMessage(`Lovely updated to ${version}`, "success");
    } catch (error) {
      addMessage(
        `Failed to update Lovely: ${
          error instanceof Error ? error.message : String(error)
        }`,
        "error",
      );
    } finally {
      lovelyUpdating = false;
    }
  }

  const ensureDownloadUrl = (mod: Mod): Mod => {
    if (mod.downloadURL && mod.downloadURL.trim().length > 0) return mod;
    if (mod._dirName) {
      return { ...mod, downloadURL: `index://${mod._dirName}` };
    }
    if (mod.id) {
      return { ...mod, downloadURL: `index://${mod.id}` };
    }
    return mod;
  };

  const installIfNeeded = async (mod?: Mod) => {
    if (!mod) return;
    if (get(installationStatus)[mod.title]) return;
    if (get(loadingStates2)[mod.title]) return;
    if (usesCatalogInstall(mod)) {
      await installCatalogEntry(mod, {
        prompt: false,
        source: settingsInstallSource(mod, get(thunderstoreEnabled), false),
      });
      return;
    }
    const withUrl = ensureDownloadUrl(mod);
    if (!withUrl.downloadURL) return;
    loadingStates2.update((s) => ({ ...s, [mod.title]: true }));
    const dependencies: string[] = [];
    if (mod.requires_steamodded) dependencies.push("Steamodded");
    if (mod.requires_talisman) dependencies.push("Talisman");
    const folderName = mod.folderName || mod.title.replace(/\s+/g, "");
    try {
      const installedPath = await invoke<string>("install_mod", {
        url: withUrl.downloadURL,
        folderName,
      });
      await invoke("add_installed_mod", {
        name: mod.title,
        path: installedPath,
        dependencies,
        currentVersion: mod.version || "",
      });
      installationStatus.update((s) => ({
        ...s,
        [mod.title]: true,
      }));
    } catch (error) {
      addMessage(
        `Failed to install ${mod.title}: ${
          error instanceof Error ? error.message : String(error)
        }`,
        "error",
      );
    } finally {
      loadingStates2.update((s) => ({ ...s, [mod.title]: false }));
    }
  };

  function handleDismiss() {
    closeDepPrompt();
  }

  async function handleAccept() {
    const state = get(depPromptStore);
    if (!state.open) return;

    const { collectionId, modTitle, modId, missing, isPreAddCheck } = state;
    closeDepPrompt();

    if (isPreAddCheck) {
      // Add the main mod to collection
      const mod = get(modsStore).find((m) => m.title === modTitle);
      setModInCollection(collectionId, modTitle, true, modId);
      await installIfNeeded(mod);
    }

    // Install missing dependencies and add them to collection
    for (const dep of missing) {
      const depMod = get(modsStore).find((m) => m.title === dep);
      setModInCollection(collectionId, dep, true, depMod?.id ?? null);
      await installIfNeeded(depMod);
    }
  }
</script>

{#if $depPromptStore.open}
  <div
    class="dep-overlay"
    role="button"
    tabindex="0"
    transition:fade={{ duration: 150 }}
    onpointerdown={handleDismiss}
    use:modal={{ onEscape: handleDismiss }}
  >
    <div
      class="dep-modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      transition:scale={{ duration: 200, start: 0.9, opacity: 0 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <h3>
        {$depPromptStore.modTitle} requires {$depPromptStore.missing.join(
          " and ",
        )}
      </h3>
      {#if $depPromptStore.isPreAddCheck}
        <p>
          Install {$depPromptStore.missing.join(" and ")} first, then add "{$depPromptStore.modTitle}"
          to "{$depPromptStore.collectionName}"?
        </p>
      {:else}
        <p>
          Add {$depPromptStore.missing.join(" and ")} to "{$depPromptStore.collectionName}"?
        </p>
      {/if}
      <div class="dep-actions">
        <button class="cancel-btn" onclick={handleDismiss}>
          {$depPromptStore.isPreAddCheck ? "Cancel" : "No"}
        </button>
        <button class="confirm-btn" onclick={handleAccept}>
          {$depPromptStore.isPreAddCheck ? "Install & Add" : "Yes"}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if $installPlanPromptStore.open && $installPlanPromptStore.plan}
  {@const plan = $installPlanPromptStore.plan}
  <div
    class="dep-overlay"
    role="button"
    tabindex="0"
    transition:fade={{ duration: 150 }}
    onpointerdown={cancelPlan}
    use:modal={{ onEscape: cancelPlan }}
  >
    <div
      class="dep-modal plan-modal"
      role="dialog"
      aria-modal="true"
      aria-label={`Install ${plan.mod_title}`}
      tabindex="-1"
      transition:scale={{ duration: 200, start: 0.9, opacity: 0 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <h3>Install {plan.mod_title}</h3>
      <p class="plan-source">{sourceLine(plan.source, rootStep?.version)}</p>

      {#if planSteps.length > 0}
        <div class="plan-section">
          <h4>Also installs</h4>
          <ul class="plan-list" data-testid="plan-steps">
            {#each planSteps as step (step.catalog_id + step.version)}
              <li>
                <span class="plan-title">{step.title}</span>
                <span class="plan-version">
                  {#if step.installed_version}
                    {step.installed_version} to {step.version || "latest"}
                  {:else}
                    {step.version || "latest"}
                  {/if}
                </span>
                {#if step.deprecated}
                  <span class="plan-tag deprecated">Deprecated</span>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if plan.satisfied.length > 0}
        <div class="plan-section">
          <h4>Already installed</h4>
          <ul class="plan-list muted">
            {#each plan.satisfied as dep (dep.full_name)}
              <li>
                <span class="plan-title">{dep.title}</span>
                <span class="plan-version">{dep.installed_version ?? ""}</span>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if plan.steamodded && !plan.steamodded.satisfied}
        <label class="plan-check">
          <input type="checkbox" bind:checked={installSteamodded} />
          <span>
            {plan.steamodded.installed
              ? `Update Steamodded (needs ${plan.steamodded.min_tag ?? plan.steamodded.min_version ?? "a newer version"}, found ${plan.steamodded.installed_version ?? "unknown"})`
              : "Install the latest Steamodded"}
          </span>
        </label>
      {/if}

      {#if plan.talisman && !plan.talisman.installed}
        <p class="plan-note">
          Needs Talisman, install it from the catalog afterwards
        </p>
      {/if}

      {#if plan.lovely && !plan.lovely.satisfied}
        <div class="plan-note lovely">
          <span>
            Needs Lovely {plan.lovely.min_version} or newer{plan.lovely
              .installed_version
              ? `, found ${plan.lovely.installed_version}`
              : ""}
          </span>
          <button
            class="plan-inline-btn"
            onclick={updateLovely}
            disabled={lovelyUpdating || lovelyUpdated}
          >
            {lovelyUpdated
              ? "Lovely updated"
              : lovelyUpdating
                ? "Updating..."
                : "Update Lovely"}
          </button>
        </div>
      {/if}

      {#if plan.warnings.length > 0}
        <ul class="plan-warnings">
          {#each plan.warnings as warning, i (i)}
            <li>{warning}</li>
          {/each}
        </ul>
      {/if}

      <div class="dep-actions">
        <button class="cancel-btn" onclick={cancelPlan}>Cancel</button>
        <button class="confirm-btn" data-autofocus onclick={confirmPlan}>
          Install
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dep-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2300;
  }

  .dep-modal {
    background: #2d2d2d;
    border: 2px solid #f4eee0;
    border-radius: 12px;
    padding: 2rem;
    width: 520px;
    max-width: 90vw;
    text-align: center;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
  }

  h3 {
    margin: 0 0 1rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.5rem;
    color: #fdcf51;
  }

  p {
    margin: 0 0 1.5rem;
    font-size: 1.15rem;
    line-height: 1.6;
    color: #f4eee0;
  }

  .dep-actions {
    display: flex;
    justify-content: center;
    gap: 1rem;
  }

  .cancel-btn,
  .confirm-btn {
    padding: 0.75rem 1.5rem;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 4px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn {
    background: #b86a2b;
    color: #f4eee0;
  }

  .cancel-btn:hover {
    background: #c97a3b;
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  .confirm-btn {
    background: #ea9600;
    color: #f4eee0;
  }

  .confirm-btn:hover {
    background: #f0a51a;
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  .confirm-btn:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  .plan-modal {
    text-align: left;
    max-height: 85vh;
    overflow-y: auto;
  }

  .plan-modal h3 {
    text-align: center;
  }

  .plan-source {
    text-align: center;
    font-size: 1rem;
    margin-bottom: 1rem;
    opacity: 0.85;
  }

  .plan-section {
    margin-bottom: 1rem;
  }

  h4 {
    margin: 0 0 0.4rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    color: #f4eee0;
  }

  .plan-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .plan-list li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.6rem;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.06);
    color: #f4eee0;
  }

  .plan-list.muted li {
    opacity: 0.7;
  }

  .plan-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .plan-version {
    font-size: 0.9rem;
    opacity: 0.8;
  }

  .plan-tag {
    font-family: "M6X11", sans-serif;
    font-size: 0.75rem;
    padding: 0.15rem 0.4rem;
    border-radius: 4px;
    text-transform: uppercase;
  }

  .plan-tag.deprecated {
    background: rgba(192, 64, 56, 0.92);
    color: #fff;
  }

  .plan-check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 1rem;
    color: #f4eee0;
    cursor: pointer;
  }

  .plan-note {
    font-size: 1rem;
    margin: 0 0 1rem;
    color: #fdcf51;
  }

  .plan-note.lovely {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .plan-inline-btn {
    padding: 0.4rem 0.8rem;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    background: #4f6367;
    color: #f4eee0;
    font-family: "M6X11", sans-serif;
    cursor: pointer;
    white-space: nowrap;
  }

  .plan-inline-btn:disabled {
    opacity: 0.7;
    cursor: default;
  }

  .plan-warnings {
    margin: 0 0 1rem;
    padding-left: 1.2rem;
    color: #fdcf51;
  }
</style>
