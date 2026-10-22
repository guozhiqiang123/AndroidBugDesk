import type { Draft, Option, Profile } from "./domain";
import type { ShortcutSettings } from "./shortcuts";
import type {
  ProjectBugDetail,
  ProjectBugAttachmentPreview,
  ProjectBugFacets,
  ProjectBugPage,
} from "./project-bugs";

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
  agentAccessEnabled: boolean;
}
export interface AiIntegrationInfo {
  platform: string;
  executablePath: string;
  bundlePath: string;
  skillPath: string;
  mcpServerName: string;
  mcpAddCommand: string;
  installPrompt: string;
  bundleVersion: string;
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
  set_shortcut_settings: ShortcutSettings;
  prepare_ai_integration: AiIntegrationInfo;
  copy_ai_install_prompt: AiIntegrationInfo;
  load_selection_memory: Partial<SelectionMemory>;
  remember_selections: void;
  list_remote_history: HistoryPage;
  list_project_bugs: ProjectBugPage;
  project_bug_facets: ProjectBugFacets;
  project_bug_workflow: Option[];
  update_project_bug_status: Option;
  project_bug_detail: ProjectBugDetail;
  preview_project_bug_attachment: ProjectBugAttachmentPreview;
  preview_project_bug_inline_file: ProjectBugAttachmentPreview;
  check_for_updates: UpdateCheckResult;
  install_update: UpdateInstallResult;
  open_release_page: void;
}
