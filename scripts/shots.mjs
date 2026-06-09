// Capture documentation screenshots of the PlayerTwo UI.
// Renders the real frontend (http://localhost:1420) in Edge with a mocked Tauri
// backend full of dummy profiles, so nothing touches your real store.
//
//   node scripts/shots.mjs
//
// Requires: the dev server running on :1420, Playwright + system Edge.

import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { mkdirSync } from "node:fs";

const outDir = join(dirname(fileURLToPath(import.meta.url)), "..", "docs", "screenshots");
mkdirSync(outDir, { recursive: true });

// ---- dummy data the mocked Tauri commands return ----
const PLATFORMS = [
  { id: "steam", name: "Steam", account_count: 3, detected: true, enabled: true },
  { id: "epic", name: "Epic Games", account_count: 3, detected: true, enabled: true },
  { id: "discord", name: "Discord", account_count: 2, detected: true, enabled: true },
  { id: "gog", name: "GOG Galaxy", account_count: 2, detected: true, enabled: true },
  { id: "riot", name: "Riot Games", account_count: 2, detected: true, enabled: true },
  { id: "ea", name: "EA Desktop", account_count: 0, detected: false, enabled: false },
  { id: "ubisoft", name: "Ubisoft Connect", account_count: 0, detected: false, enabled: false },
  { id: "battlenet", name: "Battle.net", account_count: 0, detected: false, enabled: false },
];
const ACCOUNTS = {
  steam: [
    { id: "76561198000000001", display_name: "PlayerOne", note: "Main · 1,400 games", image: null },
    { id: "76561198000000002", display_name: "Smurf", note: "Ranked alt", image: null },
    { id: "76561198000000003", display_name: "FamilyShare", note: null, image: null },
  ],
  epic: [
    { id: "epic-main", display_name: "MainAccount", note: "Fortnite", image: null },
    { id: "epic-alt", display_name: "AltAccount", note: null, image: null },
    { id: "epic-guest", display_name: "GuestAccount", note: "Rocket League", image: null },
  ],
  discord: [
    { id: "d1", display_name: "Main", note: null, image: null },
    { id: "d2", display_name: "Mod Account", note: "server moderation", image: null },
  ],
  gog: [
    { id: "g1", display_name: "Witcher", note: null, image: null },
    { id: "g2", display_name: "Cyberpunk", note: null, image: null },
  ],
  riot: [
    { id: "r1", display_name: "NA · Valorant", note: "Immortal", image: null },
    { id: "r2", display_name: "EU · League", note: null, image: null },
  ],
};
const CURRENT = { steam: "76561198000000001", epic: "epic-main", discord: "d1", gog: "g2", riot: "r1" };
const SETTINGS = { auto_start: true, minimize_after_switch: false, debug_logging: false, platforms: {} };

const mockData = { PLATFORMS, ACCOUNTS, CURRENT, SETTINGS };

// Runs in the page before the app loads: fakes window.__TAURI_INTERNALS__.
function installMock(d) {
  let cbId = 0;
  const handlers = {
    list_platforms: () => d.PLATFORMS,
    list_accounts: (a) => d.ACCOUNTS[a.platform] ?? [],
    current_account_id: (a) => d.CURRENT[a.platform] ?? null,
    get_settings: () => d.SETTINGS,
    get_data_dir: () => "C:\\Users\\you\\AppData\\Roaming\\PlayerTwo",
    get_log_path: () => "C:\\...\\target\\debug\\playertwo.log",
    renew_active_tokens: () => null,
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => {
      const id = ++cbId;
      window[`_cb_${id}`] = cb;
      return id;
    },
    invoke: (cmd, payload) => {
      const key = cmd.startsWith("plugin:") ? null : cmd;
      if (key && key in handlers) return Promise.resolve(handlers[key](payload || {}));
      return Promise.reject(new Error(`mock: no handler for ${cmd}`));
    },
  };
}

const shot = async (page, name) => {
  await page.screenshot({ path: join(outDir, name) });
  console.log("saved", name);
};

const browser = await chromium.launch({ channel: "msedge", headless: true });
const context = await browser.newContext({
  viewport: { width: 1024, height: 680 },
  deviceScaleFactor: 2,
  colorScheme: "dark",
});
await context.addInitScript(installMock, mockData);
const page = await context.newPage();

await page.goto("http://localhost:1420", { waitUntil: "networkidle" });
await page.getByText("Steam", { exact: true }).first().waitFor({ timeout: 15000 });
await page.waitForTimeout(600);

// 1) All accounts (with the first-run welcome banner)
await shot(page, "all-accounts.png");

// hide the intro for the rest
await page.evaluate(() => localStorage.setItem("seenIntro", "1"));
await page.reload({ waitUntil: "networkidle" });
await page.getByText("Steam", { exact: true }).first().waitFor({ timeout: 15000 });
await page.waitForTimeout(500);

// 2) Single platform view
await page.getByText("Epic Games", { exact: true }).first().click();
await page.waitForTimeout(500);
await shot(page, "single-platform.png");

// 3) Settings — Program tab
await page.locator('[aria-label="settings"]').click();
await page.waitForTimeout(500);
await shot(page, "settings-program.png");

// 4) Settings — Platforms tab
await page.getByRole("tab", { name: "Platforms" }).click();
await page.waitForTimeout(500);
await shot(page, "settings-platforms.png");

await browser.close();
console.log("done →", outDir);
