// Typed wrappers around the Rust commands in src-tauri/src.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface SystemInfo {
  elevated: boolean;
  osBuild: number;
  osName: string;
  mica: boolean;
  appVersion: string;
  /** "portable" covers the standalone exe and dev builds, which can't update themselves. */
  installKind: "nsis" | "msi" | "portable";
  userProfile: string;
}

export interface Drive {
  letter: string;
  root: string;
  label: string;
  fileSystem: string;
  kind: "fixed" | "removable" | "network" | "optical" | "ram";
  totalBytes: number;
  freeBytes: number;
}

export interface Program {
  id: string;
  name: string;
  version: string | null;
  publisher: string | null;
  installDate: string | null;
  estimatedSize: number | null;
  installLocation: string | null;
  inferredLocation: string | null;
  uninstallString: string | null;
  quietUninstallString: string | null;
  displayIcon: string | null;
  url: string | null;
  comments: string | null;
  registryPath: string;
  source: "machine64" | "machine32" | "user";
  isMsi: boolean;
}

export interface UninstallOutcome {
  exitCode: number | null;
  stillInstalled: boolean;
}

export type LeftoverKind = "folder" | "file" | "registryKey" | "registryValue";
export type Confidence = "high" | "medium" | "low";

export interface Leftover {
  kind: LeftoverKind;
  path: string;
  valueName: string | null;
  sizeBytes: number;
  confidence: Confidence;
  reason: string;
}

export interface RemovalResult {
  path: string;
  valueName: string | null;
  ok: boolean;
  error: string | null;
}

export interface ScanProgress {
  files: number;
  dirs: number;
  bytes: number;
  current: string;
}

export interface ScanSummary {
  rootPath: string;
  rootId: number;
  totalBytes: number;
  files: number;
  dirs: number;
  errors: number;
  durationMs: number;
  cancelled: boolean;
}

export interface NodeItem {
  id: number;
  name: string;
  size: number;
  files: number;
  isDir: boolean;
  childCount: number;
  modified: number;
  inaccessible: boolean;
}

export interface NodeDetails {
  node: NodeItem;
  path: string;
  breadcrumbs: { id: number; name: string }[];
  children: NodeItem[];
  hiddenCount: number;
  hiddenBytes: number;
}

export interface TreemapNode {
  id: number;
  name: string;
  size: number;
  isDir: boolean;
  other: number;
  children: TreemapNode[];
}

export type FileCategory =
  | "video"
  | "images"
  | "audio"
  | "archives"
  | "diskImages"
  | "programs"
  | "documents"
  | "gameData"
  | "code"
  | "logsAndTemp"
  | "data"
  | "other";

export interface FileItem {
  id: number;
  name: string;
  path: string;
  size: number;
  modified: number;
  category: FileCategory;
}

export interface TypeStat {
  extension: string;
  category: FileCategory;
  bytes: number;
  count: number;
}

export interface CleanupTarget {
  id: string;
  name: string;
  category: "system" | "browsers" | "apps" | "developer";
  description: string;
  requiresAdmin: boolean;
  defaultSelected: boolean;
}

export interface TargetReport {
  id: string;
  bytes: number;
  files: number;
  found: boolean;
  blocked: boolean;
  running: boolean;
}

export interface CleanReport {
  id: string;
  freedBytes: number;
  deletedFiles: number;
  failedFiles: number;
}

export interface AppxApp {
  name: string;
  displayName: string;
  packageFullName: string;
  publisher: string;
  version: string;
  installLocation: string | null;
  fromStore: boolean;
  known: boolean;
  recommended: boolean;
  note: string | null;
}

export interface Tweak {
  id: string;
  name: string;
  description: string;
  category: "privacy" | "start" | "explorer";
  applied: boolean;
  requiresAdmin: boolean;
  blocked: boolean;
  restartExplorer: boolean;
}

export const api = {
  systemInfo: () => invoke<SystemInfo>("system_info"),
  listDrives: () => invoke<Drive[]>("list_drives"),
  relaunchAsAdmin: () => invoke<void>("relaunch_as_admin"),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  /** A folder Explorer's "Scan with Reclaim" menu asked for, taken once. */
  takePendingScan: () => invoke<string | null>("take_pending_scan"),
  onScanRequested: (handler: () => void) => listen("scan-requested", handler),

  listPrograms: () => invoke<Program[]>("list_programs"),
  programExists: (id: string) => invoke<boolean>("program_exists", { id }),
  measurePrograms: (items: { id: string; path: string }[]) =>
    invoke<Record<string, number>>("measure_programs", { items }),
  programIcon: (id: string) => invoke<string | null>("program_icon", { id }),
  runUninstaller: (id: string, quiet = false) =>
    invoke<UninstallOutcome>("run_uninstaller", { id, quiet }),
  scanLeftovers: (query: {
    programId: string;
    name: string;
    publisher: string | null;
    installLocation: string | null;
    inferredLocation: string | null;
  }) => invoke<Leftover[]>("scan_leftovers", { query }),
  removeLeftovers: (items: Leftover[]) => invoke<RemovalResult[]>("remove_leftovers", { items }),
  registryBackupDir: () => invoke<string>("registry_backup_dir"),

  startScan: (path: string, onProgress: (p: ScanProgress) => void) => {
    const channel = new Channel<ScanProgress>();
    channel.onmessage = onProgress;
    return invoke<ScanSummary>("start_scan", { path, onProgress: channel });
  },
  cancelScan: () => invoke<void>("cancel_scan"),
  scanNode: (id: number, limit?: number) => invoke<NodeDetails>("scan_node", { id, limit }),
  scanTreemap: (id: number, depth?: number) => invoke<TreemapNode>("scan_treemap", { id, depth }),
  scanLargestFiles: (limit?: number) => invoke<FileItem[]>("scan_largest_files", { limit }),
  scanFileTypes: () => invoke<TypeStat[]>("scan_file_types"),
  deleteScanItem: (id: number, permanent: boolean) =>
    invoke<number>("delete_scan_item", { id, permanent }),

  cleanupTargets: () => invoke<CleanupTarget[]>("cleanup_targets"),
  analyzeCleanup: (ids: string[], onReport: (r: TargetReport) => void) => {
    const channel = new Channel<TargetReport>();
    channel.onmessage = onReport;
    return invoke<TargetReport[]>("analyze_cleanup", { ids, onReport: channel });
  },
  runCleanup: (ids: string[]) => invoke<CleanReport[]>("run_cleanup", { ids }),

  listAppx: () => invoke<AppxApp[]>("list_appx"),
  removeAppx: (packageFullName: string) => invoke<void>("remove_appx", { packageFullName }),
  appxLogo: (installLocation: string) => invoke<string | null>("appx_logo", { installLocation }),
  listTweaks: () => invoke<Tweak[]>("list_tweaks"),
  setTweak: (id: string, enabled: boolean) => invoke<void>("set_tweak", { id, enabled }),
  restartExplorer: () => invoke<void>("restart_explorer"),
};

/** Commands reject with the Rust error message as a plain string. */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return "Something went wrong.";
}
