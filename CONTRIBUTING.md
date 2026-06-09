# Contributing to PlayerTwo

Thanks for helping out! The most common contribution is **adding or fixing a platform
definition**, so that's documented in detail below.

## Dev setup

See the [README](README.md#build-from-source) for prerequisites (Rust, MSVC build tools,
Node 20+). Then:

```bash
npm install
npm run tauri dev      # full app
npm run build          # frontend typecheck + build only (no Rust)
```

## Project layout

```
src/                       React + MUI frontend
src-tauri/src/
  switcher/                OS-independent core
    model.rs               schema types (PlatformDef, LoginArtifact, UniqueId, ExeLocator)
    engine.rs              generic switch algorithm
    steam.rs / epic.rs     platform-specific handling
    store.rs / settings.rs persistence
  os/                      portability seam — trait `Host` (windows.rs, stub.rs)
  defs/builtin.json        all platform definitions (edit this to add a platform)
  commands.rs              Tauri command surface
```

Every OS-specific operation lives behind the `Host` trait in `os/`. Don't reach for
`std::process`, the registry, or `%VARS%` outside it — add a `Host` method instead, so
Linux/macOS support stays a matter of writing one new `Host` impl.

## Adding a platform

Add an object to the array in `src-tauri/src/defs/builtin.json`:

```jsonc
{
  "id": "myplatform",                 // stable slug, also the store folder name
  "name": "My Platform",              // shown in the UI
  "exe_default": "%ProgramFiles%\\My Platform\\app.exe",
  "exe_locators": [                   // tried before exe_default; first existing wins
    { "via": "url_protocol", "scheme": "myplatform" },
    { "via": "registry", "key": "HKCU\\Software\\My Platform", "value": "InstallPath", "suffix": "\\app.exe" },
    { "via": "app_paths", "exe": "app.exe" },
    { "via": "path", "path": "%ProgramFiles%\\My Platform\\app.exe" }
  ],
  "exe_args": null,                   // e.g. "-silent"
  "exes_to_end": ["app.exe", "app-helper.exe"],
  "login": [                          // what constitutes a login
    { "kind": "file", "live": "%AppData%\\MyPlatform\\session.dat", "saved": "session.dat" },
    { "kind": "registry", "key": "HKCU\\Software\\My Platform", "value": "Token", "saved": "Token" }
  ],
  "clear": [],                        // extra live paths to delete on logout (login files are cleared automatically)
  "unique_id": { "type": "registry", "key": "HKCU\\Software\\My Platform", "value": "AccountId" }
}
```

### Field reference

- **`%VARS%`** in paths are expanded by the `Host` (e.g. `%AppData%`, `%LocalAppData%`,
  `%ProgramFiles%`, `%ProgramFiles(x86)%`). `live` file paths may contain `*` wildcards.
- **`exe_locators`** (`ExeLocator`): `path` · `registry` (+ optional `suffix`) ·
  `app_paths` (Windows "App Paths") · `url_protocol` (reads `HKCR\<scheme>`; also used to
  detect UWP/Store apps).
- **`login`** (`LoginArtifact`): `file` (`live` → `saved` relative path) or `registry`
  (`key` + `value` → `saved` name). `saved` is where it's stored inside the account folder.
- **`unique_id`** (`UniqueId`): how to identify who's logged in now —
  - `registry` `{ key, value }`
  - `file_regex` `{ file, regex }` (one capture group; **no look-behind** — Rust `regex`)
  - `json_field` `{ file, pointer }` (RFC-6901 JSON pointer)
  - `generated_file` `{ file }` (a marker we create when no natural id exists)

Steam and Epic don't use the generic engine — see `steam.rs` / `epic.rs` and the
branches in `commands.rs`.

### Verify before submitting

- Test on a machine with the platform installed: detection, capture, switch, relaunch.
- Note any caveats in the PR (e.g. "needs remember-me", tokens rotate, etc.).
- Add a brand color/icon in `src/platformIcons.tsx` if simple-icons has one (don't add
  trademarked logos that aren't already in simple-icons without checking licensing).
- Add a one-line plain-language note to `platformInfo` in `src/platformIcons.tsx`.

## Guidelines

- **License:** PlayerTwo is GPL-3.0-or-later. Contributions are accepted under the same
  license. Don't copy code/assets from incompatibly-licensed projects.
- Keep OS-specific code behind `Host`.
- Run `npm run build` (frontend) and `cargo build` (in `src-tauri`) before opening a PR.
- Be honest about platform caveats — a half-working switch should say so in the UI note.
