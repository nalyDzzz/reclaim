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

export type LeftoverKind = "folder" | "file" | "registryKey" | "registryValue" | "service" | "scheduledTask" | "firewallRule";
export type Confidence = "high" | "medium" | "low";

export interface Leftover {
  kind: LeftoverKind;
  path: string;
  valueName: string | null;
  sizeBytes: number;
  confidence: Confidence;
  reason: string;
  /** What to show instead of `path` when that's only an ID, like a service's display name. */
  label: string | null;
}

export interface RemovalResult {
  path: string;
  valueName: string | null;
  ok: boolean;
  /** Failed only because the Recycle Bin can't take it; deleting permanently would work. */
  tooBigForRecycleBin: boolean;
  /** Failed only because File Explorer has the app's shell extension loaded; restarting Explorer frees it. */
  heldByShell: boolean;
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
  /** "large" holds the bigger cleanups, each of which gives something up. */
  category: "large" | "system" | "browsers" | "apps" | "developer";
  description: string;
  requiresAdmin: boolean;
  defaultSelected: boolean;
  /** What cleaning it gives up, said before anything happens. */
  givesUp: string | null;
}

export interface TargetReport {
  id: string;
  bytes: number;
  files: number;
  found: boolean;
  blocked: boolean;
  running: boolean;
  /** More about what's there, worth knowing before cleaning it. */
  note: string | null;
  /** A folder worth looking through before it's gone. */
  reveal: string | null;
}

export interface CleanReport {
  id: string;
  freedBytes: number;
  deletedFiles: number;
  failedFiles: number;
  /** Why a cleanup that isn't a file-by-file delete stopped or fell short. */
  error: string | null;
}

/** Sent while `runCleanup` works through the targets. */
export interface CleanStep {
  id: string;
  /** Set once the target is done. Until then it's being cleaned. */
  report: CleanReport | null;
  /** How far along a long cleanup is, from 0 to 1, when Windows says. */
  progress: number | null;
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
  /** Installed for the signed-in user. When false, only Windows' copy for new accounts is left. */
  installed: boolean;
  /** Windows keeps a copy it sets up for new accounts and can put back after a feature update. */
  provisioned: boolean;
  /** Other apps rely on it, so Reclaim leaves Windows' copy alone. */
  keepsCopy: boolean;
}

/** Debloat changes that came undone, usually after a Windows update. */
export interface Drift {
  /** Settings the user turned on that are off again. */
  tweaks: Tweak[];
  /** Removed Store apps that are installed again. */
  apps: AppxApp[];
}

export interface StartupItem {
  /** Where the entry is; pass it back to `setStartupItem`. */
  id: string;
  name: string;
  /** The entry's own name: the registry value, file, task, or service. */
  entry: string;
  publisher: string | null;
  command: string;
  program: string | null;
  source: "registry" | "folder" | "task" | "service";
  allUsers: boolean;
  enabled: boolean;
  requiresAdmin: boolean;
  blocked: boolean;
  /** The program it starts is gone, usually left behind by an uninstall. */
  missing: boolean;
  /** A service that waits until shortly after startup. */
  delayed: boolean;
}

export interface Tweak {
  id: string;
  name: string;
  description: string;
  category: "privacy" | "start" | "taskbar" | "explorer" | "gaming" | "system";
  applied: boolean;
  requiresAdmin: boolean;
  blocked: boolean;
  restartExplorer: boolean;
  /** Windows only reads it when the user signs in. */
  signOut: boolean;
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  /** Tweaks it turns on; ones this version of Windows lacks are left out. */
  tweaks: string[];
  /** Removes the commonly removed Store apps. */
  removeApps: boolean;
}

/** A Debloat setup saved to or read from a file. */
export interface Setup {
  tweaks: string[];
  /** Package names of removed Store apps. */
  apps: string[];
  /** Tweaks in the file this version of Windows, or of Reclaim, doesn't have. */
  unknownTweaks: number;
}

export interface RestorePoint {
  created: boolean;
  /** Why there isn't a new one, when there isn't. */
  note: string | null;
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
  removeLeftovers: (items: Leftover[], permanent = false) => invoke<RemovalResult[]>("remove_leftovers", { items, permanent }),
  releaseFromShell: (paths: string[]) => invoke<void>("release_from_shell", { paths }),
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
  runCleanup: (ids: string[], onStep: (s: CleanStep) => void) => {
    const channel = new Channel<CleanStep>();
    channel.onmessage = onStep;
    return invoke<CleanReport[]>("run_cleanup", { ids, onStep: channel });
  },

  listAppx: () => invoke<AppxApp[]>("list_appx"),
  /** Removes the app for this user and, as administrator, Windows' copy too. Pass no full name for a copy-only app. */
  removeAppx: (name: string, packageFullName: string | null) => invoke<void>("remove_appx", { name, packageFullName }),
  appxLogo: (installLocation: string) => invoke<string | null>("appx_logo", { installLocation }),
  listTweaks: () => invoke<Tweak[]>("list_tweaks"),
  setTweak: (id: string, enabled: boolean) => invoke<void>("set_tweak", { id, enabled }),
  restartExplorer: () => invoke<void>("restart_explorer"),
  listPresets: () => invoke<Preset[]>("list_presets"),
  exportSetup: (path: string) => invoke<Setup>("export_setup", { path }),
  readSetup: (path: string) => invoke<Setup>("read_setup", { path }),
  /** Needs administrator rights, and can take a minute. */
  createRestorePoint: () => invoke<RestorePoint>("create_restore_point"),
  checkDrift: () => invoke<Drift>("check_drift"),
  /** Stops watching these, keeping things as they are now. */
  forgetDrift: (tweaks: string[], apps: string[]) => invoke<void>("forget_drift", { tweaks, apps }),

  listStartup: () => invoke<StartupItem[]>("list_startup"),
  setStartupItem: (id: string, enabled: boolean) => invoke<void>("set_startup_item", { id, enabled }),
  fileIcon: (path: string) => invoke<string | null>("file_icon", { path }),
};

/** Commands reject with the Rust error message as a plain string. */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return "Something went wrong.";
}
