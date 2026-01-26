export interface Platform {
  id: string;
  os: string;
  osIcon: string;
  arch: string;
  archFull: string;
  variant?: string;
  displayName: string;
}
export interface Asset {
  filename: string;
  platform: Platform;
  type: "binary" | "deb" | "rpm";
  extension: string;
  size?: number;
  sha256?: string;
}
export interface AssetGroup {
  title: string;
  description: string;
  assets: Asset[];
}
export interface BuildStatus {
  build?: "success" | "failure" | "cancelled" | "skipped";
  "package-deb"?: "success" | "failure" | "cancelled" | "skipped";
  "package-rpm"?: "success" | "failure" | "cancelled" | "skipped";
}
export interface ReleaseContext {
  version: string;
  versionClean: string;
  projectName: string;
  projectDescription: string;
  repository: string;
  repositoryUrl: string;
  isPrerelease: boolean;
  isNightly: boolean;
  date: string;
  dateIso: string;
  binaries: Asset[];
  debPackages: Asset[];
  rpmPackages: Asset[];
  linuxAssets: Asset[];
  macosAssets: Asset[];
  windowsAssets: Asset[];
  hasFailures: boolean;
  buildStatus: BuildStatus;
  workflowRunUrl?: string;
  installCommands: InstallCommand[];
}
export interface InstallCommand {
  os: string;
  icon: string;
  methods: InstallMethod[];
}
export interface InstallMethod {
  name: string;
  command: string;
  note?: string;
}
export declare const PLATFORM_PATTERNS: Record<string, Partial<Platform>>;
export declare const DEB_ARCH_MAP: Record<string, string>;
export declare const RPM_ARCH_MAP: Record<string, string>;
