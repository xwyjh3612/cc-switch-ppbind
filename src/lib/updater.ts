import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";

export type UpdateChannel = "stable" | "beta";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
  releaseUrl?: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

interface PpbindUpdatePayload {
  currentVersion: string;
  availableVersion: string;
  notes?: string | null;
  pubDate?: string | null;
  releaseUrl: string;
  downloadUrl: string;
  assetName: string;
  sha256?: string | null;
}

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForUpdate(
  opts: CheckOptions = {},
): Promise<
  { status: "up-to-date" } | { status: "available"; info: UpdateInfo }
> {
  // Timeout/channel are kept for caller compatibility. The Rust command owns
  // the PPBind GitHub request and always targets PPBind's own releases.
  void opts;
  const update = await invoke<PpbindUpdatePayload | null>(
    "check_ppbind_update",
  );

  if (!update) {
    return { status: "up-to-date" };
  }

  return {
    status: "available",
    info: {
      currentVersion: update.currentVersion || (await getCurrentVersion()),
      availableVersion: update.availableVersion,
      notes: update.notes ?? undefined,
      pubDate: update.pubDate ?? undefined,
      releaseUrl: update.releaseUrl,
    },
  };
}
