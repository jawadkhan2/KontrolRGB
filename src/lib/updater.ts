import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

// Thin wrapper around the Tauri updater plugin so SettingsPage stays UI-only.
// The plugin hits the GitHub-releases `latest.json` endpoint configured in
// tauri.conf.json, verifies the bundle signature against the embedded pubkey,
// then downloads + installs and restarts.

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "uptodate" }
  | { kind: "available"; version: string; notes?: string }
  | { kind: "downloading"; pct: number }
  | { kind: "installing" }
  | { kind: "error"; message: string };

/** Check GitHub releases for a newer signed build. Returns the Update handle
 *  when one is available, or null when already up to date. */
export async function checkForUpdate(): Promise<Update | null> {
  return await check();
}

/** Download + install an update, reporting download progress, then relaunch
 *  into the new version. Never resolves on success (the process restarts). */
export async function installUpdate(
  update: Update,
  onProgress: (status: UpdateStatus) => void,
): Promise<void> {
  let total = 0;
  let received = 0;

  await update.downloadAndInstall((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress({ kind: "downloading", pct: 0 });
        break;
      case "Progress":
        received += event.data.chunkLength;
        onProgress({
          kind: "downloading",
          pct: total > 0 ? Math.round((received / total) * 100) : 0,
        });
        break;
      case "Finished":
        onProgress({ kind: "installing" });
        break;
    }
  });

  await relaunch();
}
