import { writable } from "svelte/store";
import type { MultiplayerStatus, Track } from "../utils/multiplayer";

/** Last `multiplayer_status` result, shared by the Multiplayer tab and the Installed cards */
export const multiplayerStatusStore = writable<MultiplayerStatus | null>(null);

/** Set by an Installed card, the Multiplayer tab opens the uninstall dialog for that track */
export const multiplayerUninstallRequest = writable<{ track: Track } | null>(
  null,
);
