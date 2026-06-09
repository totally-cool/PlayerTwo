#!/usr/bin/env node
// Import TcNo Account Switcher profiles into PlayerTwo's store.
//
// TcNo layout (per platform, under <tcno>/LoginCache/<Platform Name>/):
//   ids.json                     { "<uniqueId>": "<displayName>", ... }
//   <displayName>/<files...>     saved login files
//   <displayName>/reg.json       { "REG:<key>:<valueName>": "<data>", ... }
//
// Our layout (under <data>/accounts/<platformId>/):
//   accounts.json                [ { id, display_name, note, image }, ... ]
//   <uniqueId>/<files...>        saved login files (verbatim names)
//   <uniqueId>/registry.json     { "<valueName>": "<data>", ... }
//
// Usage:
//   node scripts/import-tcno.mjs [--platform epic] [--from DIR] [--to DIR] [--dry-run]

import fs from "node:fs";
import path from "node:path";
import os from "node:os";

// TcNo "LoginCache" folder name -> our platform id (must match src/defs/*.json).
const PLATFORMS = {
  epic: { tcno: "Epic Games", id: "epic" },
  gog: { tcno: "GOG Galaxy", id: "gog" },
  discord: { tcno: "Discord", id: "discord" },
};

const APPDATA =
  process.env.APPDATA || path.join(os.homedir(), "AppData", "Roaming");

function parseArgs(argv) {
  const a = { platform: "epic", dryRun: false };
  for (let i = 2; i < argv.length; i++) {
    const v = argv[i];
    if (v === "--from") a.from = argv[++i];
    else if (v === "--to") a.to = argv[++i];
    else if (v === "--platform") a.platform = argv[++i];
    else if (v === "--dry-run") a.dryRun = true;
    else if (v === "--help" || v === "-h") a.help = true;
  }
  return a;
}

const defaultTcnoDir = () => path.join(APPDATA, "TcNo Account Switcher");

// Honor the app's saved data-dir override marker if present, else default.
function defaultOurDataDir() {
  const marker = path.join(APPDATA, "PlayerTwo", "data_path.txt");
  try {
    const p = fs.readFileSync(marker, "utf8").trim();
    if (p) return p;
  } catch {}
  return path.join(APPDATA, "PlayerTwo");
}

// Match the Rust store's folder-name sanitization.
const sanitize = (id) => id.replace(/[\\/:*?"<>|]/g, "_");

// "REG:HKCU\\...\\Identifiers:AccountId" -> "AccountId"
function regSavedName(tcnoKey) {
  const k = tcnoKey.startsWith("REG:") ? tcnoKey.slice(4) : tcnoKey;
  const idx = k.lastIndexOf(":");
  return idx >= 0 ? k.slice(idx + 1) : k;
}

function copyDir(src, dst) {
  fs.mkdirSync(dst, { recursive: true });
  for (const ent of fs.readdirSync(src, { withFileTypes: true })) {
    const s = path.join(src, ent.name);
    const d = path.join(dst, ent.name);
    if (ent.isDirectory()) copyDir(s, d);
    else fs.copyFileSync(s, d);
  }
}

function help() {
  console.log(`Import TcNo Account Switcher profiles into PlayerTwo.

Usage: node scripts/import-tcno.mjs [options]

  --platform <name>  one of: ${Object.keys(PLATFORMS).join(", ")}   (default: epic)
  --from <dir>       TcNo data dir      (default: %APPDATA%\\TcNo Account Switcher)
  --to <dir>         our data dir       (default: app's configured dir, else %APPDATA%\\PlayerTwo)
  --dry-run          report only; write nothing
  -h, --help         this help
`);
}

function main() {
  const args = parseArgs(process.argv);
  if (args.help) return help();

  const map = PLATFORMS[args.platform];
  if (!map) {
    console.error(
      `Unknown platform '${args.platform}'. Known: ${Object.keys(PLATFORMS).join(", ")}`,
    );
    process.exit(1);
  }

  const fromRoot = args.from || defaultTcnoDir();
  const toRoot = args.to || defaultOurDataDir();
  const srcDir = path.join(fromRoot, "LoginCache", map.tcno);
  const idsPath = path.join(srcDir, "ids.json");
  const destPlat = path.join(toRoot, "accounts", map.id);
  const accountsFile = path.join(destPlat, "accounts.json");

  console.log(`Importing '${map.tcno}' -> platform '${map.id}'`);
  console.log(`  from: ${srcDir}`);
  console.log(`  to:   ${destPlat}`);
  if (args.dryRun) console.log("  (dry run — nothing will be written)");

  if (!fs.existsSync(idsPath)) {
    console.error(`\nNo ids.json at:\n  ${idsPath}\nNothing to import.`);
    process.exit(1);
  }

  const ids = JSON.parse(fs.readFileSync(idsPath, "utf8"));

  // Merge into any existing accounts.json so re-runs are idempotent.
  let accounts = [];
  try {
    accounts = JSON.parse(fs.readFileSync(accountsFile, "utf8"));
  } catch {}

  let imported = 0;
  let skipped = 0;

  for (const [id, name] of Object.entries(ids)) {
    const accSrc = path.join(srcDir, name); // TcNo names folders by display name
    if (!fs.existsSync(accSrc)) {
      console.warn(`  ! ${name} (${id}): source folder missing — skipped`);
      skipped++;
      continue;
    }

    const accDest = path.join(destPlat, sanitize(id));
    const registry = {};
    const entries = fs.readdirSync(accSrc, { withFileTypes: true });
    let fileCount = 0;

    for (const ent of entries) {
      const sp = path.join(accSrc, ent.name);
      if (ent.name.toLowerCase() === "reg.json") {
        const reg = JSON.parse(fs.readFileSync(sp, "utf8"));
        for (const [k, v] of Object.entries(reg)) registry[regSavedName(k)] = v;
      } else if (ent.isDirectory()) {
        fileCount++;
        if (!args.dryRun) copyDir(sp, path.join(accDest, ent.name));
      } else {
        fileCount++;
        if (!args.dryRun) {
          fs.mkdirSync(accDest, { recursive: true });
          fs.copyFileSync(sp, path.join(accDest, ent.name));
        }
      }
    }

    if (!args.dryRun) {
      fs.mkdirSync(accDest, { recursive: true });
      fs.writeFileSync(
        path.join(accDest, "registry.json"),
        JSON.stringify(registry, null, 2),
      );
    }

    const meta = { id, display_name: name, note: null, image: null };
    const idx = accounts.findIndex((a) => a.id === id);
    if (idx >= 0) accounts[idx] = meta;
    else accounts.push(meta);

    console.log(
      `  + ${name} (${id})  [${fileCount} file(s), ${Object.keys(registry).length} reg value(s)]`,
    );
    imported++;
  }

  if (!args.dryRun) {
    fs.mkdirSync(destPlat, { recursive: true });
    fs.writeFileSync(accountsFile, JSON.stringify(accounts, null, 2));
  }

  console.log(
    `\nDone. Imported ${imported}, skipped ${skipped}. Total accounts now: ${accounts.length}.`,
  );
  if (args.dryRun) console.log("Re-run without --dry-run to apply.");
}

main();
