export type ConnectionStatus = "disconnected" | "connecting" | "connected" | "error";

export interface VipUser {
  userId: string;
  login: string;
  displayName: string;
  watchStreak: number | null;
  wroteThisStream: boolean;
}

export interface StreamInfo {
  id: string;
  startedAt: string;
  complete: boolean;
}

export interface ReportUser extends VipUser {
  selected: boolean;
}

export interface ActivityEntry {
  id: number;
  kind: "vip_add" | "vip_remove" | "system" | "warning";
  title: string;
  detail: string;
  createdAt: string;
}

export interface AppSnapshot {
  configured: boolean;
  connectionStatus: ConnectionStatus;
  channelLogin: string | null;
  channelDisplayName: string | null;
  stream: StreamInfo | null;
  vipCount: number;
  vips: VipUser[];
  report: ReportUser[];
  reportComplete: boolean;
  activities: ActivityEntry[];
  streakThreshold: number;
  lastError: string | null;
}

export interface ConnectRequest {
  clientId: string;
  accessToken: string;
}
