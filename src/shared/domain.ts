import { z } from "zod";

const text = z.string().max(50_000);
export const idSchema = z.string().regex(/^[A-Za-z0-9_-]{1,128}$/);
export const profileSchema = z.object({
  name: z.string().min(1).max(100),
  edition: z.literal("central"),
  organizationId: idSchema,
  projectId: idSchema,
  businessProfiles: z.array(z.string().min(1).max(100)).min(1),
  requiredFieldIds: z.array(idSchema).default([]),
  fieldMappings: z
    .record(
      idSchema,
      z.enum([
        "subject",
        "description",
        "assignedTo",
        "sprint",
        "versions",
        "verifier",
        "participants",
        "labels",
        "trackers",
      ]),
    )
    .default({}),
  enableVersions: z.boolean().default(true),
  enableSprints: z.boolean().default(true),
  attachmentWarningMb: z.number().positive().default(50),
});
export type Profile = z.infer<typeof profileSchema>;
export const sourceSchema = z.object({
  status: z.enum(["collected", "failed", "manual", "not_applicable"]),
  at: text,
  detail: text,
});
export type Source = z.infer<typeof sourceSchema>;
export const attachmentSchema = z.object({
  id: z.uuid(),
  name: text,
  path: text,
  mime: text,
  size: z.number().nonnegative(),
  sha256: z.string().regex(/^[a-f0-9]{64}$/),
  role: z.enum(["log", "image", "video", "report", "sample"]),
  source: text,
  coverageStart: text.default(""),
  coverageEnd: text.default(""),
  confirmed: z.boolean(),
  derivedFrom: z.string().nullable().default(null),
});
export type Attachment = z.infer<typeof attachmentSchema>;
export const draftSchema = z.object({
  id: z.uuid(),
  sessionId: z.uuid().nullable(),
  title: text,
  kind: z.enum([
    "功能",
    "UI",
    "多语言",
    "动态过程",
    "崩溃",
    "ANR",
    "性能",
    "网络",
    "数据",
  ]),
  business: text,
  workitemTypeId: text,
  assignedTo: text,
  sprint: text,
  versions: z.array(text),
  buildUrl: text,
  buildStatus: z.enum([
    "unconfirmed",
    "manual_confirmed",
    "verified",
    "conflict",
  ]),
  buildManifest: z
    .object({
      packageName: text,
      versionName: text,
      versionCode: text,
      commit: text.nullable(),
      artifactSha256: text.nullable(),
    })
    .nullable(),
  app: z.object({
    packageName: text,
    versionName: text,
    versionCode: text,
    channel: text,
    environment: text,
    obfuscation: text,
    hardening: text,
  }),
  device: z.object({
    model: text,
    androidVersion: text,
    totalMemKb: text,
    availableMemKb: text,
    resolution: text,
    density: text,
  }),
  event: z.object({
    occurredAt: text,
    originalAt: text,
    source: z.enum(["auto", "manual"]),
    sourceNote: text,
    lateCapture: z.boolean(),
  }),
  preconditions: text,
  steps: text,
  expected: text,
  actual: text,
  frequency: z.object({
    firstDiscovery: z.boolean(),
    attempts: text,
    occurrences: text,
  }),
  cloud: z.object({
    applicability: z.enum(["yes", "no", "unset"]),
    reason: text,
    name: text,
    effectiveValue: text,
    source: text,
    observedAt: text,
    override: text,
  }),
  network: z.object({
    operation: text,
    type: text,
    error: text,
    collectionStatus: text,
  }),
  data: z.object({ type: text, count: text, source: text }),
  customFields: z.record(idSchema, z.union([text, z.array(text)])),
  sources: z.record(z.string(), sourceSchema),
  manualNote: text,
  collectionErrors: z.array(text),
  attachments: z.array(attachmentSchema),
  previewConfirmed: z.boolean(),
  createdAt: text,
  updatedAt: text,
});
export type Draft = z.infer<typeof draftSchema>;
export interface Option {
  id: string;
  name: string;
}
export interface Field {
  id: string;
  name: string;
  format: string;
  type: string;
  required: boolean;
  showWhenCreate?: boolean;
  optionError?: string;
  options: Option[];
}
export interface Metadata {
  profile: Profile;
  typeId: string;
  types: Option[];
  fields: Field[];
  members: Option[];
  versions: Option[];
  sprints: Option[];
  fetchedAt: string;
}
export interface ValidationIssue {
  field: string;
  message: string;
}
export type SubmissionState =
  | "draft"
  | "ready"
  | "creating"
  | "create_unknown"
  | "created"
  | "uploading"
  | "finalizing"
  | "partial"
  | "submitted";
export interface UploadRecord {
  localId: string;
  name: string;
  sha256: string;
  remoteId?: string;
  state: "pending" | "uploading" | "unknown" | "confirmed";
  error?: string;
}
export interface Evidence {
  id: string;
  directory: string;
  zipPath: string;
  sha256: string;
  files: Attachment[];
}
export interface Submission {
  draftId: string;
  profile: Profile;
  state: SubmissionState;
  evidence: Evidence;
  workitemId?: string;
  identifier?: string;
  url?: string;
  uploads: UploadRecord[];
  error?: string;
  intendedAt: string;
  updatedAt: string;
}
export interface Device {
  serial: string;
  state: string;
  label: string;
}
export interface Snapshot {
  device: Draft["device"];
  app: Partial<Draft["app"]>;
  sources: Record<string, Source>;
  errors: string[];
  details: Record<string, unknown>;
  deviceTime: string | null;
  capturedAt: string;
}
export interface Session {
  id: string;
  serial: string;
  packageName: string;
  buildUrl: string;
  business: string;
  startedAt: string;
  stoppedAt: string | null;
  snapshot: Snapshot;
  logFiles: string[];
  gaps: string[];
}
export function emptyDraft(id: string, now = new Date().toISOString()): Draft {
  return {
    id,
    sessionId: null,
    title: "",
    kind: "功能",
    business: "",
    workitemTypeId: "",
    assignedTo: "",
    sprint: "",
    versions: [],
    buildUrl: "",
    buildStatus: "unconfirmed",
    buildManifest: null,
    app: {
      packageName: "",
      versionName: "",
      versionCode: "",
      channel: "",
      environment: "",
      obfuscation: "",
      hardening: "",
    },
    device: {
      model: "",
      androidVersion: "",
      totalMemKb: "",
      availableMemKb: "",
      resolution: "",
      density: "",
    },
    event: {
      occurredAt: "",
      originalAt: "",
      source: "manual",
      sourceNote: "",
      lateCapture: false,
    },
    preconditions: "",
    steps: "",
    expected: "",
    actual: "",
    frequency: { firstDiscovery: true, attempts: "", occurrences: "" },
    cloud: {
      applicability: "unset",
      reason: "",
      name: "",
      effectiveValue: "",
      source: "",
      observedAt: "",
      override: "",
    },
    network: { operation: "", type: "", error: "", collectionStatus: "" },
    data: { type: "", count: "", source: "" },
    customFields: {},
    sources: {},
    manualNote: "",
    collectionErrors: [],
    attachments: [],
    previewConfirmed: false,
    createdAt: now,
    updatedAt: now,
  };
}
