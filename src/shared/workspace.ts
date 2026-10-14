import type { Draft, Option, Profile } from "./domain";

export interface UserIdentity {
  id: string;
  name: string;
  nickName?: string;
  avatarPng?: number[] | null;
  lastOrganization: string;
}
export interface AuthResult {
  status: "valid" | "missing" | "invalid" | "unverified";
  user: UserIdentity | null;
  message: string;
}
export interface HistoryItem {
  id: string;
  identifier: string;
  title: string;
  status: string;
  createdAt: string;
  updatedAt: string;
}
export interface HistoryPage {
  items: HistoryItem[];
  page: number;
  hasMore: boolean;
}
export type SelectionMemory = Pick<
  Draft,
  | "workitemTypeId"
  | "assignedTo"
  | "sprint"
  | "versions"
  | "kind"
  | "customFields"
>;
export interface ProjectSetting {
  projectId: string;
  enabled: boolean;
  packageName: string;
}
export interface ProjectSettings {
  entries: ProjectSetting[];
}
export interface DeveloperOptions {
  debugEnabled: boolean;
}
export interface DraftProjectScope {
  organizationId: string;
  projectId: string;
}
export interface UpdateCheckResult {
  ok: boolean;
  currentVersion: string;
  latestVersion: string | null;
  hasUpdate: boolean;
  releaseUrl: string;
  publishedAt: string | null;
  releasesChecked: number;
  checkedAtEpochMs: number;
  assetName: string | null;
  assetSizeBytes: number | null;
  canAutoUpdate: boolean;
  message: string;
  error: string | null;
}
export interface UpdateInstallResult {
  ok: boolean;
  message: string;
  filePath: string | null;
  error: string | null;
}
export interface UpdateProgress {
  phase: "connecting" | "downloading" | "verifying" | "preparing";
  downloadedBytes: number;
  totalBytes: number;
  percent: number | null;
  bytesPerSecond: number;
}
export interface WorkspaceCommands {
  check_auth: AuthResult;
  login: AuthResult;
  oauth_login: AuthResult;
  cancel_oauth_login: void;
  list_organizations: Option[];
  list_projects: Option[];
  select_project: Profile;
  load_project_settings: ProjectSettings;
  save_project_settings: ProjectSettings;
  set_developer_options: DeveloperOptions;
  load_selection_memory: Partial<SelectionMemory>;
  remember_selections: void;
  list_remote_history: HistoryPage;
  check_for_updates: UpdateCheckResult;
  install_update: UpdateInstallResult;
  open_release_page: void;
}
