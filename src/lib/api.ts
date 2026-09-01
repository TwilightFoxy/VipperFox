import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppSnapshot, ConnectRequest } from "./types";

export async function getSnapshot(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("get_snapshot");
}

export async function connectTwitch(request: ConnectRequest): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("connect_twitch", { request });
}

export async function disconnectTwitch(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("disconnect_twitch");
}

export async function removeVips(userIds: string[]): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("remove_vips", { userIds });
}

export async function setStreakThreshold(value: number): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("set_streak_threshold", { value });
}

export async function refreshVips(): Promise<AppSnapshot> {
  return invoke<AppSnapshot>("refresh_vips");
}

export function onSnapshot(handler: (snapshot: AppSnapshot) => void): Promise<UnlistenFn> {
  return listen<AppSnapshot>("snapshot-updated", (event) => handler(event.payload));
}
