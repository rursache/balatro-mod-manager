import { invoke } from "@tauri-apps/api/core";

export type LaunchResult =
  | { launched: true; usesSteam: boolean }
  | {
      launched: false;
      usesSteam: boolean;
      reason: "already_running" | "steam_not_running";
    };

// Shared by the Launch button and the Multiplayer view, the caller shows the UI
export async function launchBalatro(isLinux: boolean): Promise<LaunchResult> {
  let usesSteam = false;
  if (isLinux) {
    const running = await invoke<boolean>("check_balatro_running");
    if (running) {
      return { launched: false, usesSteam, reason: "already_running" };
    }
  }

  const path = await invoke("get_balatro_path");
  if (path && path.toString().includes("Steam")) {
    usesSteam = true;
    const running = await invoke<boolean>("check_balatro_running");
    if (running) {
      return { launched: false, usesSteam, reason: "already_running" };
    }
    if (!isLinux) {
      const steamRunning = await invoke<boolean>("check_steam_running");
      if (!steamRunning) {
        return { launched: false, usesSteam, reason: "steam_not_running" };
      }
    }
  }

  await invoke("launch_balatro");
  return { launched: true, usesSteam };
}
