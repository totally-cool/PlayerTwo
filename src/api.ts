import { invoke } from "@tauri-apps/api/core";

/** Mirror of the Rust types exposed over the command boundary. */
export interface PlatformSummary {
  id: string;
  name: string;
  account_count: number;
  detected: boolean;
  enabled: boolean;
}

export interface PlatformSettings {
  enabled: boolean | null;
  silent: boolean | null;
}

export interface Settings {
  auto_start: boolean;
  minimize_after_switch: boolean;
  debug_logging: boolean;
  platforms: Record<string, PlatformSettings>;
}

export interface Account {
  id: string;
  display_name: string;
  note: string | null;
  image: string | null;
}

export interface SwitchOutcome {
  switched: boolean;
  already_active: boolean;
  launched: boolean;
  message: string;
}

export interface AddResult {
  exists: boolean;
  account: Account;
}

/** Typed wrappers around the Tauri commands (see src-tauri/src/commands.rs). */
export const api = {
  listPlatforms: () => invoke<PlatformSummary[]>("list_platforms"),

  listAccounts: (platform: string) =>
    invoke<Account[]>("list_accounts", { platform }),

  switchAccount: (platform: string, accountId: string, autoStart: boolean) =>
    invoke<SwitchOutcome>("switch_account", {
      platform,
      accountId,
      autoStart,
    }),

  currentAccountId: (platform: string) =>
    invoke<string | null>("current_account_id", { platform }),

  prepareNewLogin: (platform: string) =>
    invoke<boolean>("prepare_new_login", { platform }),

  renewActiveTokens: () => invoke<void>("renew_active_tokens"),

  addCurrentAccount: (platform: string, displayName: string) =>
    invoke<AddResult>("add_current_account", { platform, displayName }),

  updateAccount: (platform: string, account: Account) =>
    invoke<void>("update_account", { platform, account }),

  forgetAccount: (platform: string, accountId: string) =>
    invoke<void>("forget_account", { platform, accountId }),

  getDataDir: () => invoke<string>("get_data_dir"),

  setDataDir: (path: string) => invoke<void>("set_data_dir", { path }),

  getSettings: () => invoke<Settings>("get_settings"),

  updateSettings: (settings: Settings) => invoke<void>("update_settings", { settings }),

  setPlatformEnabled: (platform: string, enabled: boolean) =>
    invoke<void>("set_platform_enabled", { platform, enabled }),

  setPlatformSilent: (platform: string, silent: boolean) =>
    invoke<void>("set_platform_silent", { platform, silent }),

  getLogPath: () => invoke<string>("get_log_path"),

  openLog: () => invoke<void>("open_log"),
};
