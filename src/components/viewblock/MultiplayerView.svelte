<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import {
    CircleArrowUp,
    CircleCheck,
    CircleX,
    Download,
    ExternalLink,
    Info,
    Play,
    RefreshCw,
    ShieldAlert,
    Square,
    SquareCheck,
    Trash2,
    TriangleAlert,
    Users,
  } from "lucide-svelte";
  import { addMessage } from "$lib/stores";
  import { openExternal } from "$lib/opener";
  import { isLinuxPlatform } from "$lib/platform";
  import LaunchAlertBox from "../LaunchAlertBox.svelte";
  import { BET_DOWNLOAD_URL } from "../../constants/links";
  import { lovelyPopupStore } from "../../stores/modStore";
  import {
    multiplayerStatusStore,
    multiplayerUninstallRequest,
  } from "../../stores/multiplayerStatus";
  import { modal } from "../../utils/modalStack";
  import { launchBalatro } from "../../utils/launch";
  import {
    TRACKS,
    defaultTrack,
    gamemodesFor,
    isModeSelectable,
    legacyMigration,
    lovelyCheck,
    primaryAction,
    removalLabel,
    steamoddedMinimum,
    trackInfo,
    type GameModeInfo,
    type LovelyRequirement,
    type MultiplayerInstallResult,
    type MultiplayerStatus,
    type MultiplayerUninstallResult,
    type PackagePlan,
    type ReadinessCheck,
    type Removal,
    type Track,
    type TrackPlan,
  } from "../../utils/multiplayer";

  type Busy = "install" | "uninstall" | "migrate" | "refresh" | "lovely";

  type Confirm =
    | {
        kind: "install";
        track: Track;
        gamemodes: string[];
        remove: Removal[];
        steamodded: string | null;
        installSteamodded: boolean;
      }
    | {
        kind: "migrate";
        steamodded: string | null;
        installSteamodded: boolean;
      }
    | { kind: "uninstall"; track: Track; removes: string[]; keeps: Removal[] };

  const TRACK_NOTES: Record<Track, string> = {
    legacy: "The classic Multiplayer mod, casual lobbies without extra modes",
    api: "The new Multiplayer API with game modes, needs Steam running",
  };

  let status = $state<MultiplayerStatus | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let busy = $state<Busy | null>(null);
  let selectedTrack = $state<Track>("api");
  let trackChosen = false;
  let selectedModes = $state<string[]>([]);
  let confirm = $state<Confirm | null>(null);
  let lovelyRequirement = $state<LovelyRequirement | null>(null);
  let lovelyInstalled = $state<boolean | null>(null);
  let launching = $state(false);
  let brokenIcons = $state<string[]>([]);
  let showSteamAlert = $state(false);
  let destroyed = false;

  let selectedInfo = $derived(status ? trackInfo(status, selectedTrack) : null);
  let action = $derived(
    status ? primaryAction(status, selectedTrack, selectedModes) : null,
  );
  let migration = $derived(status ? legacyMigration(status) : null);
  // The banner and the primary button run the same upgrade for a manual legacy copy
  let upgradeInstead = $derived(
    action?.kind === "replace" && selectedTrack === "legacy" && !!migration,
  );
  let installedTrack = $derived(
    status
      ? (status.active_track ??
          status.tracks.find((t) => t.installed)?.track ??
          null)
      : null,
  );
  let readiness = $derived<ReadinessCheck[]>(
    status
      ? [...status.readiness, lovelyCheck(lovelyRequirement, lovelyInstalled)]
      : [],
  );

  function errorText(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
  }

  async function packagePlan(fullName: string): Promise<PackagePlan | null> {
    try {
      return await invoke<PackagePlan>("plan_catalog_install", {
        id: `ts:${fullName}`,
        source: "thunderstore",
      });
    } catch (error) {
      console.warn(`Could not plan ${fullName}:`, error);
      return null;
    }
  }

  async function refreshLovely(current: MultiplayerStatus) {
    const track = installedTrack ?? selectedTrack;
    const info = trackInfo(current, track);
    const [plan, installed] = await Promise.all([
      info && current.listing_available
        ? packagePlan(info.full_name)
        : Promise.resolve(null),
      invoke<boolean>("is_lovely_installed").catch(() => null),
    ]);
    if (destroyed) return;
    lovelyRequirement = plan?.lovely ?? null;
    lovelyInstalled = installed;
  }

  async function loadStatus() {
    try {
      const next = await invoke<MultiplayerStatus>("multiplayer_status");
      if (destroyed) return;
      status = next;
      multiplayerStatusStore.set(next);
      loadError = null;
      if (!trackChosen) selectedTrack = defaultTrack(next);
      selectedModes = selectedModes.filter((name) =>
        next.gamemodes.some(
          (m) =>
            m.full_name === name &&
            isModeSelectable(m, "api", next.listing_available),
        ),
      );
      void refreshLovely(next);
    } catch (error) {
      if (destroyed) return;
      loadError = errorText(error);
    } finally {
      loading = false;
    }
  }

  async function refreshListing() {
    if (busy) return;
    busy = "refresh";
    try {
      await invoke("fetch_repo_mods", { force: true });
    } catch (error) {
      addMessage(
        `Could not refresh Thunderstore: ${errorText(error)}`,
        "error",
      );
    } finally {
      await loadStatus();
      busy = null;
    }
  }

  function selectTrack(track: Track) {
    trackChosen = true;
    selectedTrack = track;
    if (status) void refreshLovely(status);
  }

  function toggleMode(mode: GameModeInfo) {
    if (!status || !mode.full_name) return;
    if (!isModeSelectable(mode, selectedTrack, status.listing_available))
      return;
    const name = mode.full_name;
    selectedModes = selectedModes.includes(name)
      ? selectedModes.filter((n) => n !== name)
      : [...selectedModes, name];
  }

  function trackTitle(track: Track): string {
    return (status && trackInfo(status, track)?.title) || "Multiplayer";
  }

  function reportWarnings(warnings: string[]) {
    for (const warning of warnings.slice(0, 3)) addMessage(warning, "warning");
  }

  function reportLovely(plans: PackagePlan[]) {
    if (plans.some((p) => p.lovely && !p.lovely.satisfied)) {
      addMessage(
        "Multiplayer needs a newer Lovely, use Update Lovely in the readiness checklist",
        "warning",
      );
    }
  }

  async function startInstall() {
    if (!status || busy || !action?.enabled) return;
    const track = selectedTrack;
    const info = trackInfo(status, track);
    if (!info) return;
    const gamemodes = gamemodesFor(status, track, selectedModes);
    busy = "install";
    try {
      const [plan, rootPlan] = await Promise.all([
        invoke<TrackPlan>("multiplayer_plan", { track, gamemodes }),
        packagePlan(info.full_name),
      ]);
      const steamodded = rootPlan ? steamoddedMinimum([rootPlan]) : null;
      const needsSteamodded =
        !!rootPlan?.steamodded && !rootPlan.steamodded.satisfied;
      if (plan.needs_confirmation || needsSteamodded) {
        confirm = {
          kind: "install",
          track,
          gamemodes,
          remove: plan.remove,
          steamodded: needsSteamodded ? (steamodded ?? "") : null,
          installSteamodded: true,
        };
        busy = null;
        return;
      }
      await runInstall(track, gamemodes, false, false);
    } catch (error) {
      addMessage(`Could not plan the install: ${errorText(error)}`, "error");
      busy = null;
    }
  }

  async function runInstall(
    track: Track,
    gamemodes: string[],
    confirmRemoval: boolean,
    installSteamodded: boolean,
  ) {
    busy = "install";
    try {
      const result = await invoke<MultiplayerInstallResult>(
        "multiplayer_install",
        { track, gamemodes, confirmRemoval, installSteamodded },
      );
      addMessage(`${trackTitle(track)} is installed`, "success");
      if (result.removed.length > 0) {
        addMessage(`Removed ${result.removed.join(", ")}`, "info");
      }
      reportWarnings(result.warnings);
      reportLovely(result.plans);
      selectedModes = [];
    } catch (error) {
      addMessage(
        `Failed to install ${trackTitle(track)}: ${errorText(error)}`,
        "error",
      );
    } finally {
      await loadStatus();
      busy = null;
    }
  }

  async function startMigration() {
    if (!status || busy || !migration?.available) return;
    const info = trackInfo(status, "legacy");
    busy = "migrate";
    const plan = info ? await packagePlan(info.full_name) : null;
    if (plan?.steamodded && !plan.steamodded.satisfied) {
      confirm = {
        kind: "migrate",
        steamodded: steamoddedMinimum([plan]) ?? "",
        installSteamodded: true,
      };
      busy = null;
      return;
    }
    await runMigration(false);
  }

  async function runMigration(installSteamodded: boolean) {
    busy = "migrate";
    try {
      const result = await invoke<MultiplayerInstallResult>(
        "multiplayer_migrate_legacy",
        { installSteamodded },
      );
      addMessage(
        "Multiplayer now comes from the official Thunderstore package",
        "success",
      );
      reportWarnings(result.warnings);
      reportLovely(result.plans);
    } catch (error) {
      addMessage(`Upgrade failed: ${errorText(error)}`, "error");
    } finally {
      await loadStatus();
      busy = null;
    }
  }

  function startUninstall() {
    if (!status || busy) return;
    const info = trackInfo(status, selectedTrack);
    if (!info?.installed) return;
    // The backend decides what goes, the dialog names exactly those folders
    const preview = status.uninstall.find((p) => p.track === selectedTrack);
    confirm = {
      kind: "uninstall",
      track: selectedTrack,
      removes: (preview?.remove ?? []).map(removalLabel),
      keeps: preview?.keep ?? [],
    };
  }

  async function runUninstall(track: Track) {
    busy = "uninstall";
    try {
      const result = await invoke<MultiplayerUninstallResult>(
        "multiplayer_uninstall",
        { track },
      );
      addMessage(
        result.removed.length > 0
          ? `Removed ${result.removed.join(", ")}`
          : `${trackTitle(track)} was not installed`,
        "success",
      );
    } catch (error) {
      addMessage(
        `Failed to uninstall ${trackTitle(track)}: ${errorText(error)}`,
        "error",
      );
    } finally {
      await loadStatus();
      busy = null;
    }
  }

  // An Installed card asked to uninstall a track, confirm it here with the exact preview
  $effect(() => {
    const request = $multiplayerUninstallRequest;
    if (!request || !status || busy) return;
    multiplayerUninstallRequest.set(null);
    const current = status;
    untrack(() => {
      if (!trackInfo(current, request.track)?.installed) return;
      selectTrack(request.track);
      startUninstall();
    });
  });

  function cancelConfirm() {
    confirm = null;
  }

  async function acceptConfirm() {
    const current = confirm;
    if (!current) return;
    confirm = null;
    if (current.kind === "install") {
      await runInstall(
        current.track,
        current.gamemodes,
        current.remove.length > 0,
        current.steamodded !== null && current.installSteamodded,
      );
    } else if (current.kind === "migrate") {
      await runMigration(current.installSteamodded);
    } else {
      await runUninstall(current.track);
    }
  }

  async function updateLovely() {
    if (busy) return;
    busy = "lovely";
    try {
      const version = await invoke<string>("update_lovely_to_latest");
      addMessage(`Lovely ${version} is installed`, "success");
    } catch (error) {
      addMessage(`Failed to update Lovely: ${errorText(error)}`, "error");
    } finally {
      if (status) await refreshLovely(status);
      busy = null;
    }
  }

  async function doLaunch(isLinux: boolean) {
    try {
      const result = await launchBalatro(isLinux);
      if (result.launched) return;
      if (result.reason === "already_running") {
        addMessage("Balatro is already running", "error");
      } else {
        showSteamAlert = true;
      }
    } catch (error) {
      addMessage(`Failed to launch Balatro: ${errorText(error)}`, "error");
    }
  }

  async function launch() {
    if (launching) return;
    launching = true;
    try {
      const mode = await invoke<string>("get_launch_mode").catch(
        () => "modded",
      );
      if (mode === "vanilla") {
        addMessage(
          "Launch mode is set to Vanilla, switch it to Modded to play Multiplayer",
          "warning",
        );
        return;
      }
      const isLinux = await isLinuxPlatform().catch(() => false);
      if (!isLinux) {
        const present = await invoke<boolean>("is_lovely_installed").catch(
          () => true,
        );
        if (!present) {
          lovelyPopupStore.set({
            visible: true,
            source: "launch",
            onLaunchAnyway: () => doLaunch(isLinux),
          });
          return;
        }
      }
      await doLaunch(isLinux);
    } finally {
      // The game takes a moment to appear, this keeps double clicks from starting two
      setTimeout(() => (launching = false), 3000);
    }
  }

  function openBet() {
    void openExternal(status?.bet_download_url || BET_DOWNLOAD_URL);
  }

  function showIcon(url: string): boolean {
    return !!url && !brokenIcons.includes(url);
  }

  function iconFailed(url: string) {
    if (!brokenIcons.includes(url)) brokenIcons = [...brokenIcons, url];
  }

  function versionLabel(version: string | null): string {
    if (!version) return "unknown";
    return /^\d/.test(version) ? `v${version}` : version;
  }

  onMount(() => {
    void loadStatus();
    let unlisten: (() => void) | null = null;
    listen("installed-mods-changed", () => {
      if (!busy) void loadStatus();
    })
      .then((fn) => {
        if (destroyed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      destroyed = true;
      unlisten?.();
    };
  });
</script>

<div class="multiplayer-view default-scrollbar">
  <header class="mp-header">
    <div class="title-row">
      <h2><Users size={30} /> Multiplayer</h2>
      <span class="casual-badge">Casual and unranked play only</span>
      <button
        class="refresh-button"
        onclick={refreshListing}
        disabled={busy !== null}
        title="Check Thunderstore for new versions"
      >
        <span class:spinning={busy === "refresh"}><RefreshCw size={16} /></span>
        Refresh
      </button>
    </div>

    <div class="notice ranked-notice">
      <ShieldAlert size={22} />
      <div class="notice-body">
        <strong
          >Ranked requires the official Balatro Essentials Toolkit (BET)</strong
        >
        <p>
          BMM installs the official {status?.namespace ?? "BalatroMultiplayer"} packages
          from Thunderstore for casual lobbies only, ranked matches only work through
          BET
        </p>
        <button class="bet-button" onclick={openBet}>
          Get BET <ExternalLink size={14} />
        </button>
      </div>
    </div>

    <p class="support-note">
      <Info size={16} />
      Issues with this install should be reported to Balatro Mod Manager, not the
      Multiplayer team
    </p>
  </header>

  {#if loading}
    <div class="state-message">Loading Multiplayer…</div>
  {:else if loadError && !status}
    <div class="state-message">
      <p>Could not read the Multiplayer status: {loadError}</p>
      <button class="action-button neutral" onclick={() => loadStatus()}>
        <RefreshCw size={16} /> Retry
      </button>
    </div>
  {:else if status}
    {#if !status.listing_available}
      <div class="banner warning" role="status">
        <TriangleAlert size={18} />
        <span>
          Thunderstore is unavailable right now{status.listing_error
            ? `: ${status.listing_error}`
            : ""}, installs and updates are paused
        </span>
      </div>
    {/if}

    {#if status.conflict}
      <div class="banner error" role="alert">
        <CircleX size={18} />
        <span>
          Legacy Multiplayer and Multiplayer API are both installed, they cannot
          load together, install one track again or uninstall the other
        </span>
      </div>
    {/if}

    {#if migration}
      <div class="banner migration" data-testid="legacy-migration">
        <CircleArrowUp size={20} />
        <div class="banner-body">
          <strong>
            Multiplayer {versionLabel(migration.installedVersion)} from GitHub is
            installed
          </strong>
          <span>
            Lobbies compare versions, the official package{migration.latestVersion
              ? ` ${versionLabel(migration.latestVersion)}`
              : ""} keeps you in sync and your settings stay in place
          </span>
        </div>
        <button
          class="action-button primary"
          onclick={startMigration}
          disabled={!migration.available || busy !== null}
        >
          {busy === "migrate"
            ? "Upgrading…"
            : "Upgrade to the Thunderstore version"}
        </button>
      </div>
    {/if}

    <section>
      <h3>Track</h3>
      <div class="track-grid" role="radiogroup" aria-label="Multiplayer track">
        {#each TRACKS as track (track)}
          {@const info = trackInfo(status, track)}
          {#if info}
            <button
              class="track-card"
              class:selected={selectedTrack === track}
              role="radio"
              aria-checked={selectedTrack === track}
              onclick={() => selectTrack(track)}
            >
              <div class="track-head">
                {#if showIcon(info.icon_url)}
                  <img
                    src={info.icon_url}
                    alt=""
                    loading="lazy"
                    onerror={() => iconFailed(info.icon_url)}
                  />
                {:else}
                  <div class="icon-placeholder"><Users size={28} /></div>
                {/if}
                <div class="track-title">
                  <span class="name">{info.title}</span>
                  <span class="package">{info.full_name}</span>
                </div>
                <div class="badges">
                  {#if installedTrack === track && !status.conflict}
                    <span class="badge active">Active</span>
                  {:else if info.installed}
                    <span class="badge installed">Installed</span>
                  {/if}
                  {#if info.installed && !info.official}
                    <span class="badge manual">Manual</span>
                  {/if}
                  {#if info.disabled}
                    <span class="badge muted">Disabled</span>
                  {/if}
                  {#if info.deprecated}
                    <span class="badge danger">Deprecated</span>
                  {/if}
                </div>
              </div>
              <p class="track-note">{TRACK_NOTES[track]}</p>
              {#if info.description}
                <p class="track-description">{info.description}</p>
              {/if}
              <div class="versions">
                <span>
                  Installed
                  <strong
                    >{info.installed
                      ? versionLabel(info.installed_version)
                      : "none"}</strong
                  >
                </span>
                <span>
                  Latest
                  <strong
                    >{info.latest_version
                      ? versionLabel(info.latest_version)
                      : "unavailable"}</strong
                  >
                </span>
                {#if info.update_available}
                  <span class="update-hint">Update available</span>
                {/if}
              </div>
            </button>
          {/if}
        {/each}
      </div>

      <div class="actions">
        {#if action}
          <button
            class="action-button"
            class:primary={action.kind === "install" ||
              action.kind === "replace" ||
              action.kind === "add_modes"}
            class:update={action.kind === "update"}
            class:neutral={action.kind === "none"}
            onclick={upgradeInstead ? startMigration : startInstall}
            disabled={(upgradeInstead
              ? !migration?.available
              : !action.enabled) || busy !== null}
          >
            {#if action.kind === "update"}
              <CircleArrowUp size={18} />
            {:else}
              <Download size={18} />
            {/if}
            {#if busy === "install" || (upgradeInstead && busy === "migrate")}
              Installing…
            {:else}
              {upgradeInstead ? "Upgrade" : action.label}
            {/if}
          </button>
        {/if}
        {#if selectedInfo?.installed}
          <button
            class="action-button danger"
            onclick={startUninstall}
            disabled={busy !== null}
          >
            <Trash2 size={18} />
            {busy === "uninstall" ? "Uninstalling…" : "Uninstall"}
          </button>
        {/if}
        <button
          class="action-button launch"
          onclick={launch}
          disabled={launching || !installedTrack || status.conflict}
          title={installedTrack
            ? "Launch Balatro with your mods"
            : "Install a track first"}
        >
          <Play size={18} />
          {launching ? "Launching…" : "Launch"}
        </button>
        {#if selectedInfo?.package_url}
          <button
            class="link-button"
            onclick={() => openExternal(selectedInfo?.package_url ?? "")}
          >
            Thunderstore page <ExternalLink size={14} />
          </button>
        {/if}
      </div>
    </section>

    <section>
      <h3>Game modes</h3>
      {#if selectedTrack !== "api"}
        <p class="section-hint">Game modes need the Multiplayer API track</p>
      {/if}
      {#if status.gamemodes.length === 0}
        <p class="section-hint">No game modes are published yet</p>
      {:else}
        <div class="mode-grid">
          {#each status.gamemodes as mode (mode.full_name ?? mode.mod_id ?? mode.title)}
            {@const selectable = isModeSelectable(
              mode,
              selectedTrack,
              status.listing_available,
            )}
            {@const checked =
              mode.installed ||
              (!!mode.full_name && selectedModes.includes(mode.full_name))}
            <button
              class="mode-card"
              class:checked
              class:coming-soon={mode.coming_soon}
              role="checkbox"
              aria-checked={checked}
              disabled={!selectable}
              onclick={() => toggleMode(mode)}
            >
              <span class="mode-check">
                {#if checked}
                  <SquareCheck size={20} />
                {:else}
                  <Square size={20} />
                {/if}
              </span>
              {#if showIcon(mode.icon_url)}
                <img
                  src={mode.icon_url}
                  alt=""
                  loading="lazy"
                  onerror={() => iconFailed(mode.icon_url)}
                />
              {/if}
              <span class="mode-body">
                <span class="mode-title">
                  {mode.title}
                  {#if mode.coming_soon}
                    <span class="badge muted">Coming soon</span>
                  {:else if mode.installed}
                    <span class="badge installed">Installed</span>
                  {/if}
                  {#if mode.deprecated}
                    <span class="badge danger">Deprecated</span>
                  {/if}
                  {#if mode.update_available}
                    <span class="badge update">Update</span>
                  {/if}
                </span>
                {#if mode.description}
                  <span class="mode-description">{mode.description}</span>
                {/if}
                {#if !mode.coming_soon}
                  <span class="mode-versions">
                    {#if mode.installed}
                      Installed {versionLabel(mode.installed_version)} ·
                    {/if}
                    Latest {mode.latest_version
                      ? versionLabel(mode.latest_version)
                      : "unavailable"}
                  </span>
                {/if}
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <section>
      <h3>Readiness</h3>
      {#if !installedTrack}
        <p class="section-hint">
          Install a track to check Steamodded, OpenSSL and Steam
        </p>
      {/if}
      <ul class="checklist">
        {#each readiness as check (check.id)}
          <li class="check {check.status}">
            <span class="check-icon">
              {#if check.status === "ok"}
                <CircleCheck size={20} />
              {:else if check.status === "warning"}
                <TriangleAlert size={20} />
              {:else}
                <CircleX size={20} />
              {/if}
            </span>
            <span class="check-body">
              <span class="check-label">{check.label}</span>
              {#if check.detail}
                <span class="check-detail">{check.detail}</span>
              {/if}
            </span>
            {#if check.id === "lovely" && check.status !== "ok"}
              <button
                class="action-button small update"
                onclick={updateLovely}
                disabled={busy !== null}
              >
                {busy === "lovely" ? "Updating…" : "Update Lovely"}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

{#if confirm}
  <div
    class="confirm-overlay"
    role="button"
    tabindex="0"
    transition:fade={{ duration: 150 }}
    onpointerdown={cancelConfirm}
    use:modal={{ onEscape: cancelConfirm }}
  >
    <div
      class="confirm-modal"
      role="dialog"
      aria-modal="true"
      aria-labelledby="mp-confirm-title"
      tabindex="-1"
      transition:scale={{ duration: 200, start: 0.9, opacity: 0 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      {#if confirm.kind === "install"}
        <h3 id="mp-confirm-title">
          {confirm.remove.length > 0
            ? `Switch to ${trackTitle(confirm.track)}?`
            : `Install ${trackTitle(confirm.track)}?`}
        </h3>
        {#if confirm.remove.length > 0}
          <p>
            The two Multiplayer tracks cannot be installed together, BMM removes
            these once the install finishes
          </p>
          <ul class="removal-list">
            {#each confirm.remove as removal (removal.path)}
              <li>
                <strong>{removalLabel(removal)}</strong>
                <span>{removal.reason}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {:else if confirm.kind === "migrate"}
        <h3 id="mp-confirm-title">Upgrade to the Thunderstore version?</h3>
        <p>
          BMM installs the official package, then removes the old GitHub copy
        </p>
      {:else}
        <h3 id="mp-confirm-title">Uninstall {trackTitle(confirm.track)}?</h3>
        {#if confirm.removes.length > 0}
          <p>This removes</p>
          <ul class="removal-list">
            {#each confirm.removes as name (name)}
              <li><strong>{name}</strong></li>
            {/each}
          </ul>
        {:else}
          <p>BMM has nothing it can remove for this track</p>
        {/if}
        {#if confirm.keeps.length > 0}
          <p>These stay in the Mods folder</p>
          <ul class="removal-list">
            {#each confirm.keeps as kept (kept.path)}
              <li>
                <strong>{removalLabel(kept)}</strong>
                <span>{kept.reason}</span>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}

      {#if confirm.kind !== "uninstall" && confirm.steamodded !== null}
        <label class="steamodded-option">
          <input type="checkbox" bind:checked={confirm.installSteamodded} />
          <span>
            Also install the latest Steamodded{confirm.steamodded
              ? `, ${confirm.steamodded} or newer is required`
              : ""}
          </span>
        </label>
      {/if}

      <div class="confirm-actions">
        <button class="cancel-btn" onclick={cancelConfirm}>Cancel</button>
        <button class="confirm-btn" data-autofocus onclick={acceptConfirm}>
          {#if confirm.kind === "uninstall"}
            Uninstall
          {:else if confirm.kind === "migrate"}
            Upgrade
          {:else if confirm.remove.length > 0}
            Switch tracks
          {:else}
            Install
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<LaunchAlertBox
  show={showSteamAlert}
  onClose={() => (showSteamAlert = false)}
/>

<style>
  .multiplayer-view {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    /* Same column edges as Installed Mods: a scrollbar gutter on both sides and
       a small inset, pulled out by that inset */
    scrollbar-gutter: stable both-edges;
    margin: 0 -0.5rem;
    padding: 1.5rem 0.5rem 2.5rem;
    box-sizing: border-box;
    color: var(--ui-text);
    font-family: "M6X11", sans-serif;
  }

  .mp-header {
    margin-bottom: 1rem;
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
  }

  h2 {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 2.5rem;
    color: var(--ui-heading);
    margin: 0;
  }

  h3 {
    font-size: 1.8rem;
    color: var(--ui-heading);
    margin: 1.5rem 0 0.75rem;
  }

  .casual-badge {
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    padding: 0.35rem 0.75rem;
    border-radius: 6px;
    background: var(--ui-success);
    border: 2px solid var(--ui-mod-chip-border);
    color: var(--ui-text);
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .refresh-button {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    background: var(--ui-mod-chip-bg);
    color: var(--ui-mod-chip-text);
    border: 2px solid var(--ui-mod-chip-border);
    border-radius: 6px;
    padding: 0.5rem 0.9rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.05rem;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .refresh-button:hover:not(:disabled) {
    background: var(--ui-mod-chip-hover);
    transform: translateY(-2px);
  }

  .refresh-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .refresh-button span {
    display: inline-flex;
  }

  .spinning {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .notice {
    display: flex;
    align-items: flex-start;
    gap: 0.75rem;
    padding: 1rem 1.1rem;
    border-radius: 6px;
    background: var(--ui-note-bg);
    border: 2px solid var(--ui-note-border);
    color: var(--ui-text);
  }

  .notice > :global(svg) {
    color: var(--ui-note-icon);
    flex-shrink: 0;
    margin-top: 2px;
  }

  .notice-body {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.4rem;
  }

  .notice-body strong {
    font-size: 1.25rem;
    color: var(--ui-heading);
  }

  .notice-body p {
    margin: 0;
    font-size: 1.05rem;
    line-height: 1.4;
  }

  .bet-button {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin-top: 0.3rem;
    background: var(--ui-copy-bg);
    color: var(--ui-text);
    border: 2px solid var(--ui-copy-border);
    border-radius: 4px;
    padding: 0.4rem 0.8rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .bet-button:hover {
    background: var(--ui-copy-hover);
    border-color: var(--ui-copy-hover-border);
    transform: translateY(-2px);
  }

  .support-note {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.75rem 0 0;
    font-size: 1rem;
    color: var(--ui-text-soft);
  }

  .state-message {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 3rem 1rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.5rem;
    text-align: center;
  }

  .state-message p {
    margin: 0;
  }

  .banner {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.8rem 1rem;
    margin-top: 0.75rem;
    border-radius: 6px;
    font-size: 1.05rem;
    line-height: 1.35;
  }

  .banner > :global(svg) {
    flex-shrink: 0;
  }

  .banner.warning {
    background: var(--ui-toast-warning);
    border: 2px solid var(--ui-toast-warning-border);
  }

  .banner.error {
    background: var(--ui-toast-error);
    border: 2px solid var(--ui-danger-strong);
  }

  .banner.migration {
    background: var(--ui-toast-info);
    border: 2px solid var(--ui-info);
    flex-wrap: wrap;
  }

  .banner-body {
    flex: 1;
    min-width: 220px;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }

  .banner-body strong {
    color: var(--ui-heading);
    font-size: 1.15rem;
  }

  .track-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 1rem;
  }

  .track-card {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    text-align: left;
    padding: 1rem;
    border-radius: 8px;
    border: 2px solid var(--ui-mod-panel-border);
    background: var(--ui-mod-panel);
    color: var(--ui-text);
    font-family: inherit;
    cursor: pointer;
    box-shadow: 0 4px 0 var(--ui-mod-panel-shadow);
    transition:
      transform 0.15s ease,
      box-shadow 0.15s ease,
      border-color 0.15s ease;
  }

  .track-card:hover {
    transform: translateY(-2px);
  }

  .track-card.selected {
    border-color: var(--ui-heading);
    box-shadow:
      0 0 0 2px var(--ui-heading),
      0 4px 0 var(--ui-mod-panel-shadow);
  }

  .track-head {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .track-head img,
  .icon-placeholder {
    width: 56px;
    height: 56px;
    border-radius: 6px;
    flex-shrink: 0;
    image-rendering: pixelated;
  }

  .icon-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--ui-glass);
  }

  .track-title {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .track-title .name {
    font-family: "M6X11", sans-serif;
    font-size: 1.5rem;
    color: var(--ui-heading);
  }

  .track-title .package {
    font-size: 0.85rem;
    color: var(--ui-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badges {
    margin-left: auto;
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.3rem;
  }

  .badge {
    font-family: "M6X11", monospace;
    font-size: 0.8rem;
    padding: 0.2rem 0.45rem;
    border-radius: 4px;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    border: 1px solid rgba(255, 255, 255, 0.25);
    color: #fff;
    background: rgba(20, 20, 20, 0.72);
    white-space: nowrap;
  }

  .badge.active {
    background: var(--ui-success-strong);
  }

  .badge.installed {
    background: var(--ui-success);
  }

  .badge.update {
    background: var(--ui-info);
  }

  .badge.manual {
    background: var(--ui-neutral-warm);
  }

  .badge.danger {
    background: var(--ui-danger);
  }

  .badge.muted {
    background: var(--ui-neutral);
  }

  .track-note {
    margin: 0;
    font-size: 1.05rem;
    color: var(--ui-text);
  }

  .track-description {
    margin: 0;
    font-size: 0.95rem;
    color: var(--ui-text-soft);
    line-height: 1.3;
  }

  .versions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem 1rem;
    font-size: 0.95rem;
    color: var(--ui-text-soft);
  }

  .versions strong {
    color: var(--ui-text);
  }

  .update-hint {
    color: var(--ui-heading);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
    margin-top: 1rem;
  }

  .action-button {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    border: none;
    border-radius: 4px;
    padding: 0.7rem 1.3rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.2rem;
    color: var(--ui-text);
    background: var(--ui-neutral);
    outline: var(--ui-neutral-outline) solid 2px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .action-button.primary {
    background: var(--ui-success);
    outline-color: var(--ui-button-green-border);
  }

  .action-button.primary:hover:not(:disabled) {
    background: var(--ui-success-hover);
  }

  .action-button.update {
    background: var(--ui-info);
    outline-color: var(--ui-info-strong);
  }

  .action-button.update:hover:not(:disabled) {
    background: var(--ui-info-hover);
  }

  .action-button.danger {
    background: var(--ui-danger);
    outline-color: var(--ui-danger-outline);
  }

  .action-button.danger:hover:not(:disabled) {
    background: var(--ui-danger-hover);
  }

  .action-button.launch {
    background: var(--ui-launch-bg);
    outline-color: var(--ui-launch-outline);
  }

  .action-button.launch:hover:not(:disabled) {
    background: var(--ui-launch-hover);
  }

  .action-button:hover:not(:disabled) {
    transform: translateY(-2px);
  }

  .action-button:active:not(:disabled) {
    transform: translateY(1px);
  }

  .action-button:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .action-button.small {
    padding: 0.4rem 0.8rem;
    font-size: 1rem;
    margin-left: auto;
  }

  .link-button {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    background: none;
    border: none;
    padding: 0;
    font-family: "M6X11", sans-serif;
    font-size: 1.05rem;
    color: var(--ui-heading);
    cursor: pointer;
  }

  .link-button:hover {
    text-decoration: underline;
  }

  .section-hint {
    margin: 0 0 0.75rem;
    color: var(--ui-text-soft);
    font-size: 1.05rem;
  }

  .mode-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 0.75rem;
  }

  .mode-card {
    display: flex;
    align-items: flex-start;
    gap: 0.6rem;
    text-align: left;
    padding: 0.8rem;
    border-radius: 8px;
    border: 2px solid var(--ui-mod-panel-border);
    background: var(--ui-mod-panel);
    color: var(--ui-text);
    font-family: inherit;
    cursor: pointer;
    transition:
      transform 0.15s ease,
      border-color 0.15s ease;
  }

  .mode-card:hover:not(:disabled) {
    transform: translateY(-2px);
  }

  .mode-card.checked {
    border-color: var(--ui-heading);
  }

  .mode-card:disabled {
    cursor: not-allowed;
  }

  .mode-card:disabled:not(.checked) {
    opacity: 0.55;
  }

  .mode-card.coming-soon {
    border-style: dashed;
  }

  .mode-check {
    display: inline-flex;
    color: var(--ui-heading);
    flex-shrink: 0;
  }

  .mode-card img {
    width: 40px;
    height: 40px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .mode-body {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }

  .mode-title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.25rem;
    color: var(--ui-heading);
  }

  .mode-description {
    font-size: 0.95rem;
    color: var(--ui-text-soft);
    line-height: 1.3;
  }

  .mode-versions {
    font-size: 0.9rem;
    color: var(--ui-text-muted);
  }

  .checklist {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.7rem 0.9rem;
    border-radius: 6px;
    background: var(--ui-glass);
    border: 1px solid var(--ui-glass-border);
  }

  .check-icon {
    display: inline-flex;
    flex-shrink: 0;
  }

  .check.ok .check-icon {
    color: var(--ui-switch-on);
  }

  .check.warning .check-icon {
    color: var(--ui-warning);
  }

  .check.error .check-icon {
    color: var(--ui-switch-off);
  }

  .check-body {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    min-width: 0;
  }

  .check-label {
    font-family: "M6X11", sans-serif;
    font-size: 1.15rem;
  }

  .check-detail {
    font-size: 0.95rem;
    color: var(--ui-text-soft);
    overflow-wrap: anywhere;
  }

  .confirm-overlay {
    position: fixed;
    inset: 0;
    background: var(--ui-modal-overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2300;
  }

  .confirm-modal {
    background: var(--ui-mod-input-bg);
    border: 2px solid var(--ui-mod-panel-border);
    border-radius: 12px;
    padding: 2rem;
    width: 540px;
    max-width: 90vw;
    text-align: center;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    color: var(--ui-text);
    font-family: "M6X11", sans-serif;
  }

  .confirm-modal h3 {
    margin: 0 0 1rem;
    font-size: 1.6rem;
  }

  .confirm-modal p {
    margin: 0 0 1rem;
    font-size: 1.1rem;
    line-height: 1.5;
  }

  .removal-list {
    list-style: none;
    margin: 0 0 1rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    text-align: left;
  }

  .removal-list li {
    display: flex;
    flex-direction: column;
    padding: 0.5rem 0.75rem;
    border-radius: 6px;
    background: var(--ui-glass);
    border: 1px solid var(--ui-glass-border);
  }

  .removal-list span {
    font-size: 0.95rem;
    color: var(--ui-text-soft);
  }

  .steamodded-option {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    justify-content: center;
    margin: 0 0 1.25rem;
    font-size: 1.05rem;
    cursor: pointer;
  }

  .steamodded-option input {
    width: 18px;
    height: 18px;
    accent-color: var(--ui-success);
  }

  .confirm-actions {
    display: flex;
    justify-content: center;
    gap: 1rem;
  }

  .cancel-btn,
  .confirm-btn {
    padding: 0.75rem 1.5rem;
    border: 2px solid var(--ui-mod-chip-border);
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    color: var(--ui-text);
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 4px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn {
    background: var(--ui-neutral-warm);
  }

  .confirm-btn {
    background: var(--ui-mod-chip-bg);
  }

  .cancel-btn:hover,
  .confirm-btn:hover {
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn:active,
  .confirm-btn:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  @media (max-width: 1160px) {
    h2 {
      font-size: 2rem;
    }

    h3 {
      font-size: 1.5rem;
    }

    .action-button {
      font-size: 1.05rem;
      padding: 0.6rem 1rem;
    }
  }
</style>
