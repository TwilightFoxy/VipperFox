import { describe, expect, it } from "vitest";
import type { ReportUser } from "./types";
import {
  MAX_REMOVALS_PER_OPERATION,
  canAddSelection,
  channelConfirmationMatches,
  selectSafeBatch,
} from "./removalSafety";

function report(count: number): ReportUser[] {
  return Array.from({ length: count }, (_, index) => ({
    userId: String(index + 1),
    login: `viewer${index + 1}`,
    displayName: `Viewer ${index + 1}`,
    watchStreak: null,
    wroteThisStream: false,
    selected: false,
  }));
}

describe("removal safety", () => {
  it("requires an exact channel login ignoring only case and surrounding spaces", () => {
    expect(channelConfirmationMatches(" Fox_Channel ", "fox_channel")).toBe(true);
    expect(channelConfirmationMatches("another_channel", "fox_channel")).toBe(false);
    expect(channelConfirmationMatches("", "fox_channel")).toBe(false);
    expect(channelConfirmationMatches("fox_channel", null)).toBe(false);
  });

  it("limits select-all to one safe batch", () => {
    const selected = selectSafeBatch(report(50));
    expect(selected.size).toBe(MAX_REMOVALS_PER_OPERATION);
    expect(selected.has("21")).toBe(false);
  });

  it("blocks a new selection after the limit but permits deselection", () => {
    const selected = selectSafeBatch(report(MAX_REMOVALS_PER_OPERATION));
    expect(canAddSelection(selected, "21")).toBe(false);
    expect(canAddSelection(selected, "1")).toBe(true);
  });
});
