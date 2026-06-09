import { Fragment, useEffect, useRef, useState } from "react";
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogContentText,
  DialogActions,
  Button,
  Tabs,
  Tab,
  Box,
  Avatar,
  List,
  ListItem,
  ListItemIcon,
  ListItemText,
  ListSubheader,
  Switch,
  TextField,
  Divider,
  InputAdornment,
  IconButton,
  Tooltip,
} from "@mui/material";
import FolderOpenIcon from "@mui/icons-material/FolderOpen";
import SearchIcon from "@mui/icons-material/Search";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import ContentCopyIcon from "@mui/icons-material/ContentCopy";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { open } from "@tauri-apps/plugin-dialog";
import { api, Settings, PlatformSummary, Account } from "./api";
import { PlatformIcon, platformInfo } from "./platformIcons";

/** Header/footer bar background: paper + a hover-tint overlay, so it reads as a
 *  distinct band from the content in BOTH light and dark mode. */
const barBg = (t: any) => {
  // Use the CSS variable so the overlay tracks the active scheme (light/dark);
  // t.palette.* under cssVariables is the default-scheme static value.
  const hover = t.vars?.palette.action.hover ?? t.palette.action.hover;
  return {
    bgcolor: "background.paper",
    backgroundImage: `linear-gradient(${hover}, ${hover})`,
  };
};

/** Program + Platform settings, in a tabbed dialog. */
export function SettingsDialog(props: {
  open: boolean;
  platforms: PlatformSummary[];
  dataDir: string;
  onChanged: () => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState(0);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [logPath, setLogPath] = useState("");
  const [updateMsg, setUpdateMsg] = useState("");
  const [checking, setChecking] = useState(false);
  const [dataDirInput, setDataDirInput] = useState("");
  const [query, setQuery] = useState("");

  const applyDataDir = async () => {
    const v = dataDirInput.trim();
    if (!v || v === props.dataDir) return;
    try {
      await api.setDataDir(v);
      props.onChanged();
    } catch {
      /* ignore */
    }
  };

  // Browse only fills the field; nothing is applied until "Set" is clicked.
  const browseDataDir = async () => {
    try {
      const selected = await open({ directory: true, defaultPath: dataDirInput || props.dataDir });
      if (typeof selected === "string") setDataDirInput(selected);
    } catch {
      /* cancelled / unavailable */
    }
  };

  const dataDirModified = dataDirInput.trim() !== "" && dataDirInput.trim() !== props.dataDir;

  const checkUpdates = async () => {
    setChecking(true);
    setUpdateMsg("");
    try {
      const u = await check();
      if (u) {
        setUpdateMsg(`Update ${u.version} found — downloading…`);
        await u.downloadAndInstall();
        await relaunch();
      } else {
        setUpdateMsg("You're up to date.");
      }
    } catch {
      setUpdateMsg("Update check failed (updater not configured or offline).");
    } finally {
      setChecking(false);
    }
  };

  useEffect(() => {
    if (props.open) {
      api.getSettings().then(setSettings).catch(() => {});
      api.getLogPath().then(setLogPath).catch(() => {});
      setDataDirInput(props.dataDir);
    }
  }, [props.open, props.dataDir]);

  const saveSettings = async (next: Settings) => {
    setSettings(next);
    try {
      await api.updateSettings(next);
      props.onChanged();
    } catch {
      /* ignore */
    }
  };

  const togglePlatform = async (id: string, enabled: boolean) => {
    try {
      await api.setPlatformEnabled(id, enabled);
      props.onChanged();
    } catch {
      /* ignore */
    }
  };

  const toggleSilent = async (id: string, silent: boolean) => {
    if (settings) {
      const prev = settings.platforms?.[id] ?? { enabled: null, silent: null };
      setSettings({ ...settings, platforms: { ...settings.platforms, [id]: { ...prev, silent } } });
    }
    try {
      await api.setPlatformSilent(id, silent);
      props.onChanged();
    } catch {
      /* ignore */
    }
  };

  const sorted = [...props.platforms].sort((a, b) => a.name.localeCompare(b.name));
  const filtered = sorted.filter((p) =>
    p.name.toLowerCase().includes(query.trim().toLowerCase()),
  );

  return (
    <Dialog open={props.open} onClose={props.onClose} fullWidth maxWidth="sm">
      <DialogTitle sx={barBg}>Settings</DialogTitle>
      <Tabs
        value={tab}
        onChange={(_, v) => setTab(v)}
        variant="fullWidth"
        sx={(t) => ({ ...barBg(t), borderBottom: 1, borderColor: "divider" })}
      >
        <Tab label="Program" />
        <Tab label="Platforms" />
      </Tabs>
      <DialogContent
        dividers
        sx={(t) => {
          const hover = t.vars?.palette.action.hover ?? t.palette.action.hover;
          const paper = t.vars?.palette.background.paper ?? t.palette.background.paper;
          return {
            p: 0,
            bgcolor: "background.paper",
            "& .MuiListSubheader-root": {
              backgroundColor: paper,
              backgroundImage: `linear-gradient(${hover}, ${hover})`,
            },
          };
        }}
      >
        {tab === 0 && settings && (
          <List dense disablePadding>
            <ListSubheader>Data</ListSubheader>
            <ListItem>
              <Box sx={{ width: "100%", display: "flex", flexDirection: "column", gap: 1 }}>
                <TextField
                  fullWidth
                  label="Data location"
                  value={dataDirInput}
                  onChange={(e) => setDataDirInput(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && applyDataDir()}
                  helperText="Local path, mapped drive, or \\NAS\share — accounts are stored here"
                />
                <Box sx={{ display: "flex", gap: 2, justifyContent: "flex-end" }}>
                  <Button startIcon={<FolderOpenIcon />} onClick={browseDataDir}>
                    Browse…
                  </Button>
                  <Button
                    variant={dataDirModified ? "contained" : "text"}
                    onClick={applyDataDir}
                    disabled={!dataDirModified}
                  >
                    Set
                  </Button>
                </Box>
              </Box>
            </ListItem>

            <Divider component="li" />
            <ListSubheader>Switching</ListSubheader>
            <ListItem
              secondaryAction={
                <Switch
                  edge="end"
                  checked={settings.auto_start}
                  onChange={(e) => saveSettings({ ...settings, auto_start: e.target.checked })}
                />
              }
            >
              <ListItemText
                primary="Launch app after switching"
                secondary="Start the platform once the account is swapped"
              />
            </ListItem>
            <Divider variant="middle" component="li" />
            <ListItem
              secondaryAction={
                <Switch
                  edge="end"
                  checked={settings.minimize_after_switch}
                  onChange={(e) =>
                    saveSettings({ ...settings, minimize_after_switch: e.target.checked })
                  }
                />
              }
            >
              <ListItemText
                primary="Minimize after switching"
                secondary="Hide the window after a successful switch"
              />
            </ListItem>

            <Divider component="li" />
            <ListSubheader>Diagnostics</ListSubheader>
            <ListItem
              secondaryAction={
                <Switch
                  edge="end"
                  checked={settings.debug_logging}
                  onChange={(e) => saveSettings({ ...settings, debug_logging: e.target.checked })}
                />
              }
            >
              <ListItemText
                primary="Debug logging"
                secondary="Verbose logs (applies on restart)"
              />
            </ListItem>
            <ListItem sx={{ pl: 9, py: 0, gap: 1 }}>
              <Tooltip title={logPath || "…"}>
                <Button size="small" onClick={() => api.openLog()}>
                  Open log file
                </Button>
              </Tooltip>
              <Tooltip title="Copy log file path">
                <IconButton
                  size="small"
                  aria-label="copy log path"
                  onClick={() => navigator.clipboard?.writeText(logPath).catch(() => {})}
                >
                  <ContentCopyIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            </ListItem>
            <Divider variant="middle" component="li" />
            <ListItem
              secondaryAction={
                <Button onClick={checkUpdates} disabled={checking}>
                  {checking ? "Checking…" : "Check"}
                </Button>
              }
            >
              <ListItemText primary="Updates" secondary={updateMsg || "Check for a new version"} />
            </ListItem>
          </List>
        )}

        {tab === 1 && (
          <List dense subheader={<li />} sx={{ pt: 0 }}>
            <Box
              component="li"
              sx={(t) => ({
                ...barBg(t),
                position: "sticky",
                top: 0,
                zIndex: 2,
                p: 1,
                listStyleType: "none",
                borderBottom: 1,
                borderColor: "divider",
              })}
            >
              <TextField
                fullWidth
                size="small"
                placeholder="Search platforms…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                slotProps={{
                  input: {
                    startAdornment: (
                      <InputAdornment position="start">
                        <SearchIcon fontSize="small" />
                      </InputAdornment>
                    ),
                  },
                }}
              />
            </Box>
            {filtered.map((p, i) => {
              const silent = settings?.platforms?.[p.id]?.silent ?? true;
              // Group platforms sharing a base name (e.g. the Discord variants):
              // a full-bleed divider appears only when the family changes.
              const family = (n: string) => n.split(/[\s:]+/)[0].toLowerCase();
              const newFamily = i > 0 && family(filtered[i - 1].name) !== family(p.name);
              const hasOptions = p.id === "epic" && p.enabled;
              return (
                <Fragment key={p.id}>
                  {newFamily && <Divider component="li" />}
                  <ListItem
                    secondaryAction={
                      <Switch
                        edge="end"
                        checked={p.enabled}
                        onChange={(e) => togglePlatform(p.id, e.target.checked)}
                      />
                    }
                  >
                    <ListItemIcon sx={{ minWidth: 40 }}>
                      <PlatformIcon platformId={p.id} size={22} brandColor />
                    </ListItemIcon>
                    <ListItemText
                      primary={
                        <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.5 }}>
                          {p.name}
                          <Tooltip title={platformInfo(p.id)}>
                            <InfoOutlinedIcon
                              sx={{ fontSize: 16, color: "text.secondary", cursor: "help" }}
                            />
                          </Tooltip>
                        </Box>
                      }
                      secondary={p.detected ? "Detected on this PC" : "Not detected"}
                    />
                  </ListItem>
                  {hasOptions && (
                    <ListItem
                      sx={{ pl: 9, py: 0 }}
                      secondaryAction={
                        <Switch
                          edge="end"
                          size="small"
                          checked={silent}
                          onChange={(e) => toggleSilent(p.id, e.target.checked)}
                        />
                      }
                    >
                      <ListItemText
                        primary="Launch silently"
                        secondary="Start Epic with -silent after switching"
                        slotProps={{
                          primary: { variant: "body2" },
                          secondary: { variant: "caption" },
                        }}
                      />
                    </ListItem>
                  )}
                </Fragment>
              );
            })}
          </List>
        )}
      </DialogContent>
      <DialogActions sx={barBg}>
        <Button onClick={props.onClose}>Close</Button>
      </DialogActions>
    </Dialog>
  );
}

/** Load an image file and downscale it to a small square JPEG data URL. */
function fileToAvatar(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const img = new Image();
      img.onerror = () => reject(new Error("Unsupported image"));
      img.onload = () => {
        const size = 128;
        const canvas = document.createElement("canvas");
        canvas.width = size;
        canvas.height = size;
        const ctx = canvas.getContext("2d");
        if (!ctx) return reject(new Error("no canvas context"));
        const scale = Math.max(size / img.width, size / img.height);
        const w = img.width * scale;
        const h = img.height * scale;
        ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
        resolve(canvas.toDataURL("image/jpeg", 0.85));
      };
      img.src = reader.result as string;
    };
    reader.readAsDataURL(file);
  });
}

/** Per-account settings: avatar, display name, note, and delete (with confirm). */
export function AccountSettingsDialog(props: {
  open: boolean;
  account: Account | null;
  onClose: () => void;
  onSave: (name: string, note: string, image: string | null) => void;
  onDelete: () => void;
}) {
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [image, setImage] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (props.open && props.account) {
      setName(props.account.display_name);
      setNote(props.account.note ?? "");
      setImage(props.account.image ?? null);
      setConfirm(false);
    }
  }, [props.open, props.account]);

  const onFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (file) {
      try {
        setImage(await fileToAvatar(file));
      } catch {
        /* ignore unsupported file */
      }
    }
  };

  return (
    <Dialog open={props.open} onClose={props.onClose} fullWidth maxWidth="sm">
      <DialogTitle>Account settings</DialogTitle>
      <DialogContent>
        <Box sx={{ display: "flex", flexDirection: "column", gap: 2, mt: 1 }}>
          <Box sx={{ display: "flex", gap: 2, alignItems: "center" }}>
            <Avatar src={image ?? undefined} sx={{ width: 64, height: 64 }}>
              {name.charAt(0).toUpperCase()}
            </Avatar>
            <Box>
              <Button size="small" onClick={() => fileRef.current?.click()}>
                Choose image…
              </Button>
              {image && (
                <Button size="small" color="inherit" onClick={() => setImage(null)}>
                  Remove
                </Button>
              )}
            </Box>
            <input ref={fileRef} type="file" accept="image/*" hidden onChange={onFile} />
          </Box>
          <TextField
            label="Display name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            fullWidth
            autoFocus
          />
          <TextField
            label="Note"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            fullWidth
            multiline
            minRows={2}
          />
        </Box>
      </DialogContent>
      <DialogActions>
        <Button color="error" sx={{ mr: "auto" }} onClick={() => setConfirm(true)}>
          Delete profile
        </Button>
        <Button onClick={props.onClose}>Cancel</Button>
        <Button variant="contained" onClick={() => props.onSave(name.trim(), note.trim(), image)}>
          Save
        </Button>
      </DialogActions>

      <Dialog open={confirm} onClose={() => setConfirm(false)} maxWidth="xs" fullWidth>
        <DialogTitle>Delete profile?</DialogTitle>
        <DialogContent>
          <DialogContentText>
            Remove “{props.account?.display_name}” from the switcher? This deletes its saved login
            files here. Your actual account on the platform is not affected.
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConfirm(false)}>Cancel</Button>
          <Button
            color="error"
            variant="contained"
            onClick={() => {
              setConfirm(false);
              props.onDelete();
            }}
          >
            Delete
          </Button>
        </DialogActions>
      </Dialog>
    </Dialog>
  );
}
