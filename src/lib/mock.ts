import type { AppSnapshot } from "./types";

export const emptySnapshot: AppSnapshot = {
  configured: false,
  connectionStatus: "disconnected",
  channelLogin: null,
  channelDisplayName: null,
  stream: null,
  vipCount: 0,
  vips: [],
  report: [],
  reportComplete: true,
  activities: [],
  streakThreshold: 150,
  lastError: null
};

export const previewSnapshot: AppSnapshot = {
  configured: true,
  connectionStatus: "connected",
  channelLogin: "twilightfoxy_",
  channelDisplayName: "TwilightFoxy",
  stream: { id: "preview", startedAt: new Date(Date.now() - 6_842_000).toISOString(), complete: true },
  vipCount: 4,
  streakThreshold: 150,
  lastError: null,
  vips: [
    { userId: "1", login: "cgosl_los", displayName: "cgosl_los", watchStreak: 164, wroteThisStream: true },
    { userId: "2", login: "finik0000", displayName: "FiNiK0000", watchStreak: null, wroteThisStream: false },
    { userId: "3", login: "meipl_nahida_1", displayName: "meipl_nahida_1", watchStreak: 151, wroteThisStream: true },
    { userId: "4", login: "mind_of_the_nebula", displayName: "mind_of_the_nebula", watchStreak: null, wroteThisStream: false }
  ],
  report: [],
  reportComplete: true,
  activities: [
    { id: 1, kind: "vip_add", title: "meipl_nahida_1 получил VIP", detail: "Watch Streak достиг 151", createdAt: new Date(Date.now() - 420_000).toISOString() },
    { id: 2, kind: "system", title: "Мониторинг запущен", detail: "EventSub подключён без пропусков", createdAt: new Date(Date.now() - 6_842_000).toISOString() }
  ]
};
