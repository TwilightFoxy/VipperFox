import type { ReportUser } from "./types";

export const MAX_REMOVALS_PER_OPERATION = 20;

export function channelConfirmationMatches(value: string, channelLogin: string | null): boolean {
  return Boolean(channelLogin) && value.trim().toLowerCase() === channelLogin!.toLowerCase();
}

export function selectSafeBatch(report: ReportUser[]): Set<string> {
  return new Set(report.slice(0, MAX_REMOVALS_PER_OPERATION).map((item) => item.userId));
}

export function canAddSelection(selected: Set<string>, userId: string): boolean {
  return selected.has(userId) || selected.size < MAX_REMOVALS_PER_OPERATION;
}
