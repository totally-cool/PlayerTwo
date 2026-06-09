import { useEffect, useState, useCallback, type ReactNode } from "react";
import {
  AppBar,
  Toolbar,
  Typography,
  Box,
  Collapse,
  Divider,
  List,
  ListSubheader,
  ListItem,
  ListItemButton,
  ListItemAvatar,
  ListItemIcon,
  ListItemText,
  Chip,
  Card,
  CardActionArea,
  Avatar,
  Button,
  Snackbar,
  Alert,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogContentText,
  DialogActions,
  TextField,
  Tooltip,
  IconButton,
} from "@mui/material";
import { useColorScheme } from "@mui/material/styles";
import { keyframes } from "@mui/system";
import AddIcon from "@mui/icons-material/Add";
import DownloadIcon from "@mui/icons-material/Download";
import MoreVertIcon from "@mui/icons-material/MoreVert";
import ViewModuleIcon from "@mui/icons-material/ViewModule";
import ViewListIcon from "@mui/icons-material/ViewList";
import MenuIcon from "@mui/icons-material/Menu";
import MenuOpenIcon from "@mui/icons-material/MenuOpen";
import CheckCircleIcon from "@mui/icons-material/CheckCircle";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import ExpandLessIcon from "@mui/icons-material/ExpandLess";
import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import LightModeIcon from "@mui/icons-material/LightMode";
import DarkModeIcon from "@mui/icons-material/DarkMode";
import SettingsBrightnessIcon from "@mui/icons-material/SettingsBrightness";
import SettingsIcon from "@mui/icons-material/Settings";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, Account, AddResult, PlatformSummary, Settings } from "./api";
import { PlatformIcon, platformInfo, avatarColor } from "./platformIcons";
import { SettingsDialog, AccountSettingsDialog } from "./SettingsDialog";
import { UpdateNotifier } from "./UpdateNotifier";

const VIEW_KEY = "view";

// Slide-in transitions, re-triggered by changing the element's `key`.
const slideUp = keyframes`
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
`;
const slideInLeft = keyframes`
  from { opacity: 0; transform: translateX(-14px); }
  to { opacity: 1; transform: translateX(0); }
`;

/** v9 color-scheme toggle: cycles system → light → dark via `useColorScheme`. */
function ModeToggle() {
  const { mode, setMode } = useColorScheme();
  if (!mode) return null;
  const next = mode === "system" ? "light" : mode === "light" ? "dark" : "system";
  const icon =
    mode === "system" ? (
      <SettingsBrightnessIcon />
    ) : mode === "dark" ? (
      <DarkModeIcon />
    ) : (
      <LightModeIcon />
    );
  return (
    <Tooltip title={`Theme: ${mode} — click for ${next}`}>
      <IconButton color="inherit" onClick={() => setMode(next)} aria-label="toggle color scheme">
        {icon}
      </IconButton>
    </Tooltip>
  );
}

/**
 * New-profile flow. The launcher has already been opened logged-out. We sit at
 * step 0 until the user confirms they've logged in, then ask for a name. If the
 * captured account already exists, a banner offers to rename it instead.
 * Cancel is always available.
 */
function NewProfileDialog(props: {
  open: boolean;
  mode: "fresh" | "import";
  platformName?: string;
  onClose: () => void;
  onSave: (name: string) => Promise<AddResult>;
  onRename: (acc: Account) => void;
}) {
  // "fresh" starts at the wait-for-login step; "import" goes straight to naming.
  const [step, setStep] = useState<0 | 1>(0);
  const [name, setName] = useState("");
  const [exists, setExists] = useState<Account | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (props.open) {
      setStep(props.mode === "fresh" ? 0 : 1);
      setName("");
      setExists(null);
      setBusy(false);
    }
  }, [props.open, props.mode]);

  const save = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    try {
      const res = await props.onSave(name.trim());
      if (res.exists) setExists(res.account);
    } catch {
      // surfaced via parent toast
    } finally {
      setBusy(false);
    }
  };

  const title = props.mode === "fresh" ? "New profile" : "Import current login";

  return (
    <Dialog open={props.open} onClose={props.onClose} fullWidth maxWidth="sm">
      <DialogTitle>
        {title}
        {props.platformName ? ` — ${props.platformName}` : ""}
      </DialogTitle>
      <DialogContent>
        {step === 0 ? (
          <DialogContentText>
            The launcher has opened. Sign into the <b>new</b> account, and once you’re fully
            logged in, click <b>“I’ve logged in”</b>.
          </DialogContentText>
        ) : (
          <>
            <DialogContentText sx={{ mb: 2 }}>
              {props.mode === "import"
                ? "Save the account currently signed in — nothing is logged out."
                : "Name this profile to save the current login."}
            </DialogContentText>
            {exists && (
              <Alert
                severity="warning"
                sx={{ mb: 2 }}
                action={
                  <Button color="inherit" size="small" onClick={() => props.onRename(exists)}>
                    Rename it
                  </Button>
                }
              >
                That account is already saved as “{exists.display_name}”. Sign into a different
                account, or rename the existing profile.
              </Alert>
            )}
            <TextField
              autoFocus
              fullWidth
              margin="dense"
              label="Profile name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && save()}
            />
          </>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={props.onClose}>Cancel</Button>
        {step === 0 ? (
          <Button variant="contained" onClick={() => setStep(1)}>
            I’ve logged in
          </Button>
        ) : (
          <Button variant="contained" disabled={busy} onClick={save}>
            Save
          </Button>
        )}
      </DialogActions>
    </Dialog>
  );
}

export default function App() {
  const [platforms, setPlatforms] = useState<PlatformSummary[]>([]);
  const [view, setView] = useState<string>(() => localStorage.getItem(VIEW_KEY) || "all");
  const [layout, setLayout] = useState<"grid" | "list">(
    () => (localStorage.getItem("layout") as "grid" | "list") || "list",
  );
  const [nav, setNav] = useState<"drawer" | "rail">(
    () => (localStorage.getItem("nav") as "drawer" | "rail") || "drawer",
  );
  const [collapsed, setCollapsed] = useState<Set<string>>(
    () => new Set<string>(JSON.parse(localStorage.getItem("collapsed") || "[]")),
  );
  const [accountsByPlatform, setAccountsByPlatform] = useState<Record<string, Account[]>>({});
  const [currentByPlatform, setCurrentByPlatform] = useState<Record<string, string | null>>({});
  const [settings, setSettings] = useState<Settings | null>(null);
  const [dataDir, setDataDir] = useState("");
  const [toast, setToast] = useState<{ msg: string; sev: "success" | "error" | "info" } | null>(
    null,
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [showIntro, setShowIntro] = useState(() => localStorage.getItem("seenIntro") !== "1");
  const [editTarget, setEditTarget] = useState<{ platformId: string; account: Account } | null>(
    null,
  );
  const [newProfile, setNewProfile] = useState<{
    platformId: string;
    mode: "fresh" | "import";
  } | null>(null);

  const refreshAll = useCallback(async () => {
    try {
      const ps = await api.listPlatforms();
      setPlatforms(ps);
      setDataDir(await api.getDataDir());
      setSettings(await api.getSettings());
      const accs: Record<string, Account[]> = {};
      const curs: Record<string, string | null> = {};
      await Promise.all(
        ps.map(async (p) => {
          accs[p.id] = await api.listAccounts(p.id);
          try {
            curs[p.id] = await api.currentAccountId(p.id);
          } catch {
            curs[p.id] = null;
          }
        }),
      );
      setAccountsByPlatform(accs);
      setCurrentByPlatform(curs);
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  }, []);

  useEffect(() => {
    // Keep the active account's rotating token (Epic) fresh, then load.
    api.renewActiveTokens().catch(() => {});
    refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    localStorage.setItem(VIEW_KEY, view);
  }, [view]);

  useEffect(() => {
    localStorage.setItem("layout", layout);
  }, [layout]);

  useEffect(() => {
    localStorage.setItem("nav", nav);
  }, [nav]);

  useEffect(() => {
    localStorage.setItem("collapsed", JSON.stringify([...collapsed]));
  }, [collapsed]);

  const toggleCollapse = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  // Fall back to "all" if a remembered platform no longer exists.
  useEffect(() => {
    if (view !== "all" && platforms.length > 0 && !platforms.some((p) => p.id === view && p.enabled)) {
      setView("all");
    }
  }, [platforms, view]);

  const onSwitch = async (platformId: string, acc: Account) => {
    try {
      const out = await api.switchAccount(platformId, acc.id, settings?.auto_start ?? true);
      setToast({
        msg: out.already_active ? "Already active" : out.message || `Switched to ${acc.display_name}`,
        sev: out.already_active ? "info" : "success",
      });
      refreshAll();
      if (settings?.minimize_after_switch) {
        try {
          await getCurrentWindow().minimize();
        } catch {
          /* not running in a Tauri window */
        }
      }
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  };

  const onForget = async (platformId: string, acc: Account) => {
    try {
      await api.forgetAccount(platformId, acc.id);
      refreshAll();
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  };

  const onDeleteAccount = () => {
    const t = editTarget;
    setEditTarget(null);
    if (t) onForget(t.platformId, t.account);
  };

  const onSaveAccount = async (name: string, note: string, image: string | null) => {
    const t = editTarget;
    setEditTarget(null);
    if (!t) return;
    try {
      await api.updateAccount(t.platformId, {
        ...t.account,
        display_name: name || t.account.display_name,
        note: note ? note : null,
        image: image ?? null,
      });
      refreshAll();
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  };

  // "New profile": log out, open the launcher for a fresh sign-in, then capture.
  const onNewProfile = async (platformId: string) => {
    setNewProfile({ platformId, mode: "fresh" });
    try {
      await api.prepareNewLogin(platformId);
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  };

  // "Import current login": capture whoever is signed in now — no logout.
  const onImportCurrent = (platformId: string) => {
    setNewProfile({ platformId, mode: "import" });
  };

  const onSaveNew = async (name: string): Promise<AddResult> => {
    const platformId = newProfile!.platformId;
    const res = await api.addCurrentAccount(platformId, name);
    if (!res.exists) {
      setToast({ msg: `Saved ${name}`, sev: "success" });
      setNewProfile(null);
      refreshAll();
    }
    return res;
  };

  const onRenameExisting = (acc: Account) => {
    const platformId = newProfile?.platformId;
    setNewProfile(null);
    if (platformId) setEditTarget({ platformId, account: acc });
  };

  // Only enabled platforms appear in the main UI; the rest live in Settings.
  const enabledPlatforms = platforms.filter((p) => p.enabled);
  const totalCount = enabledPlatforms.reduce((s, p) => s + p.account_count, 0);
  const activePlatforms =
    view === "all" ? enabledPlatforms : enabledPlatforms.filter((p) => p.id === view);
  const newProfileName = platforms.find((p) => p.id === newProfile?.platformId)?.name;
  const detectedNames = platforms.filter((p) => p.detected).map((p) => p.name);

  // Only offer "import current" when something is logged in that isn't saved yet.
  const currentId = view !== "all" ? currentByPlatform[view] ?? null : null;
  const currentAlreadySaved =
    currentId != null && (accountsByPlatform[view] ?? []).some((a) => a.id === currentId);
  const showImport = view !== "all" && currentId != null && !currentAlreadySaved;

  // Sidebar: alphabetical, split into "has profiles" and "empty" sections.
  const sortedPlatforms = [...enabledPlatforms].sort((a, b) => a.name.localeCompare(b.name));
  const withProfiles = sortedPlatforms.filter((p) => p.account_count > 0);
  const emptyPlatforms = sortedPlatforms.filter((p) => p.account_count === 0);

  const renderPlatform = (p: PlatformSummary) => {
    const button = (
      <ListItemButton
        key={p.id}
        selected={view === p.id}
        onClick={() => setView(p.id)}
        sx={nav === "rail" ? { justifyContent: "center", px: 1, py: 1.25, my: 0.5 } : { pl: 4 }}
      >
        <ListItemIcon sx={{ minWidth: nav === "rail" ? 0 : 36, justifyContent: "center" }}>
          <PlatformIcon platformId={p.id} size={20} brandColor />
        </ListItemIcon>
        {nav === "drawer" && (
          <>
            <ListItemText primary={p.name} />
            {p.account_count > 0 ? (
              <Chip size="small" label={p.account_count} />
            ) : (
              <Tooltip title="No profiles yet — open to add one">
                <IconButton
                  size="small"
                  edge="end"
                  color="primary"
                  onClick={(e) => {
                    e.stopPropagation();
                    setView(p.id);
                  }}
                >
                  <AddIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            )}
          </>
        )}
      </ListItemButton>
    );
    return nav === "rail" ? (
      <Tooltip key={p.id} title={p.name} placement="right">
        {button}
      </Tooltip>
    ) : (
      button
    );
  };

  const renderAccountRow = (pid: string, acc: Account) => {
    const isActive = currentByPlatform[pid] === acc.id;
    const av = avatarColor(acc.id);
    return (
      <ListItem
        key={`${pid}:${acc.id}`}
        disablePadding
        secondaryAction={
          <IconButton
            edge="end"
            size="small"
            onClick={() => setEditTarget({ platformId: pid, account: acc })}
          >
            <MoreVertIcon fontSize="small" />
          </IconButton>
        }
      >
        <ListItemButton selected={isActive} onClick={() => onSwitch(pid, acc)}>
          <ListItemAvatar sx={{ minWidth: 44 }}>
            <Avatar
              src={acc.image ?? undefined}
              sx={{ width: 32, height: 32, fontSize: 14, bgcolor: av.bg, color: av.fg }}
            >
              {acc.display_name.charAt(0).toUpperCase()}
            </Avatar>
          </ListItemAvatar>
          <ListItemText primary={acc.display_name} secondary={acc.note || undefined} />
          {isActive && <CheckCircleIcon color="warning" fontSize="small" sx={{ mr: 1, opacity: 0.9 }} />}
        </ListItemButton>
      </ListItem>
    );
  };

  const renderAccountCard = (pid: string, acc: Account) => {
    const isActive = currentByPlatform[pid] === acc.id;
    const av = avatarColor(acc.id);
    return (
      <Card
        key={`${pid}:${acc.id}`}
        elevation={isActive ? 6 : 2}
        sx={{
          position: "relative",
          borderRadius: 2,
          border: isActive ? 2 : 0,
          borderStyle: "solid",
          borderColor: "primary.main",
          transition: "box-shadow 150ms ease, transform 150ms ease",
          "&:hover": { boxShadow: 8, transform: "translateY(-3px)" },
        }}
      >
        <Box sx={{ position: "absolute", top: 6, left: 8, opacity: 0.7, zIndex: 1 }}>
          <PlatformIcon platformId={pid} size={16} brandColor />
        </Box>
        <CardActionArea
          onClick={() => onSwitch(pid, acc)}
          sx={{ p: 2, pt: 3.5, display: "flex", flexDirection: "column", gap: 1 }}
        >
          <Avatar
            src={acc.image ?? undefined}
            sx={{
              width: 56,
              height: 56,
              bgcolor: isActive ? "primary.main" : av.bg,
              color: isActive ? "primary.contrastText" : av.fg,
            }}
          >
            {acc.display_name.charAt(0).toUpperCase()}
          </Avatar>
          <Typography variant="body2" noWrap sx={{ maxWidth: "100%" }}>
            {acc.display_name}
          </Typography>
          {isActive ? (
            <Chip size="small" color="warning" label="Active" />
          ) : (
            acc.note && (
              <Typography variant="caption" color="text.secondary" noWrap>
                {acc.note}
              </Typography>
            )
          )}
        </CardActionArea>
        <Tooltip title="Profile settings">
          <IconButton
            size="small"
            onClick={() => setEditTarget({ platformId: pid, account: acc })}
            sx={{ position: "absolute", top: 2, right: 2, zIndex: 1 }}
          >
            <MoreVertIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Card>
    );
  };

  // A platform = a collapsible card. Collapsed shows just the header with a
  // banner of the active profile; expanded shows all profiles.
  const renderPlatformGroup = (
    p: PlatformSummary,
    accs: Account[],
    opts: { collapsible?: boolean; footer?: ReactNode } = {},
  ) => {
    const collapsible = opts.collapsible !== false;
    const isCollapsed = collapsible && collapsed.has(p.id);
    const activeAcc = accs.find((a) => a.id === currentByPlatform[p.id]);
    return (
      <Card key={p.id} variant="outlined" sx={{ overflow: "hidden" }}>
        <ListItemButton
          onClick={collapsible ? () => toggleCollapse(p.id) : undefined}
          disableRipple={!collapsible}
          sx={{
            gap: 1.25,
            cursor: collapsible ? "pointer" : "default",
            bgcolor: "background.paper",
            backgroundImage: (t) => {
              const h = t.vars?.palette.action.hover ?? t.palette.action.hover;
              return `linear-gradient(${h}, ${h})`;
            },
            borderBottom: isCollapsed ? 0 : 1,
            borderColor: "divider",
          }}
        >
          <PlatformIcon platformId={p.id} size={18} brandColor />
          <Typography sx={{ fontWeight: 700, fontSize: "0.8125rem", color: "text.primary" }}>
            {p.name}
          </Typography>
          <Tooltip title={platformInfo(p.id)}>
            <InfoOutlinedIcon
              fontSize="small"
              onClick={(e) => e.stopPropagation()}
              sx={{ ml: 0.5, color: "text.secondary", cursor: "help" }}
            />
          </Tooltip>
          <Box sx={{ flexGrow: 1 }} />
          {isCollapsed && activeAcc ? (
            <Tooltip title={`Active: ${activeAcc.display_name}`}>
              <Chip
                size="small"
                variant="outlined"
                color="primary"
                avatar={
                  <Avatar src={activeAcc.image ?? undefined}>
                    {activeAcc.display_name.charAt(0).toUpperCase()}
                  </Avatar>
                }
                label={activeAcc.display_name}
                onClick={(e) => {
                  e.stopPropagation();
                  onSwitch(p.id, activeAcc);
                }}
                sx={{ mr: 1, maxWidth: 200 }}
              />
            </Tooltip>
          ) : (
            <Chip size="small" label={accs.length} sx={{ mr: 1 }} />
          )}
          {collapsible && (isCollapsed ? <ExpandMoreIcon /> : <ExpandLessIcon />)}
        </ListItemButton>
        <Collapse in={!isCollapsed} timeout={250} unmountOnExit>
          {layout === "list" ? (
            <List dense disablePadding>
              {accs.map((acc) => renderAccountRow(p.id, acc))}
              {opts.footer}
            </List>
          ) : (
            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
                gap: 1.5,
                p: 1.5,
              }}
            >
              {accs.map((acc) => renderAccountCard(p.id, acc))}
              {opts.footer}
            </Box>
          )}
        </Collapse>
      </Card>
    );
  };

  return (
    <Box sx={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <AppBar
        position="static"
        color="default"
        elevation={0}
        sx={{ borderBottom: 1, borderColor: "divider" }}
      >
        <Toolbar variant="dense">
          <Typography variant="h6" sx={{ flexGrow: 1 }}>
            PlayerTwo
          </Typography>
          <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
            <ModeToggle />
          </Box>
        </Toolbar>
      </AppBar>

      <Box sx={{ display: "flex", flexGrow: 1, minHeight: 0 }}>
        {/* Sidebar: navigation drawer (labels) or icon rail */}
        <Box
          sx={{
            width: nav === "rail" ? 72 : 240,
            flexShrink: 0,
            borderRight: 1,
            borderColor: "divider",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            transition: "width 240ms cubic-bezier(0.4, 0, 0.2, 1)",
          }}
        >
          <Box
            key={nav}
            sx={{
              flexGrow: 1,
              minHeight: 0,
              display: "flex",
              flexDirection: "column",
              animation: `${slideInLeft} 240ms ease`,
            }}
          >
            <List dense sx={{ flexGrow: 1, overflowY: "auto", overflowX: "hidden" }}>
            {nav === "rail" ? (
              <Tooltip title="All accounts" placement="right">
                <ListItemButton
                  selected={view === "all"}
                  onClick={() => setView("all")}
                  sx={{ justifyContent: "center", px: 1, py: 1.25, my: 0.5 }}
                >
                  <ListItemIcon sx={{ minWidth: 0, justifyContent: "center" }}>
                    <ViewModuleIcon />
                  </ListItemIcon>
                </ListItemButton>
              </Tooltip>
            ) : (
              <ListItemButton selected={view === "all"} onClick={() => setView("all")}>
                <ListItemIcon sx={{ minWidth: 36 }}>
                  <ViewModuleIcon />
                </ListItemIcon>
                <ListItemText primary="All accounts" />
                {totalCount > 0 && <Chip size="small" label={totalCount} />}
              </ListItemButton>
            )}

            {nav === "rail" && <Divider sx={{ my: 0.5 }} />}

            {nav === "drawer" && withProfiles.length > 0 && (
              <ListSubheader disableSticky>With profiles</ListSubheader>
            )}
            {withProfiles.map(renderPlatform)}

            {nav === "drawer" && emptyPlatforms.length > 0 && (
              <ListSubheader disableSticky>No profiles yet</ListSubheader>
            )}
            {emptyPlatforms.map(renderPlatform)}
          </List>
          <Divider />
          <Box
            sx={{
              display: "flex",
              flexDirection: nav === "rail" ? "column" : "row",
              alignItems: "center",
              justifyContent: nav === "rail" ? "center" : "space-between",
              gap: 0.5,
              p: 0.5,
            }}
          >
            <Tooltip title="Settings" placement="right">
              <IconButton onClick={() => setSettingsOpen(true)} aria-label="settings">
                <SettingsIcon />
              </IconButton>
            </Tooltip>
            <Tooltip
              title={nav === "drawer" ? "Collapse to icon rail" : "Expand navigation"}
              placement="right"
            >
              <IconButton
                onClick={() => setNav(nav === "drawer" ? "rail" : "drawer")}
                aria-label="toggle navigation style"
              >
                {nav === "drawer" ? <MenuOpenIcon /> : <MenuIcon />}
              </IconButton>
            </Tooltip>
          </Box>
          </Box>
        </Box>

        {/* Account grid */}
        <Box sx={{ flexGrow: 1, p: 2, overflowY: "auto" }}>
          {showIntro && (
            <Alert
              severity="info"
              variant="filled"
              sx={{ mb: 2 }}
              onClose={() => {
                setShowIntro(false);
                localStorage.setItem("seenIntro", "1");
              }}
            >
              <strong>Welcome to PlayerTwo!</strong> Auto-detected on this PC:{" "}
              {detectedNames.length ? detectedNames.join(", ") : "none yet"}. Enable others in
              Settings → Platforms.
              <br />
              Capture a profile while you're signed in, then click any card to switch. Hover a
              platform's ⓘ to see how its switching works.
            </Alert>
          )}
          <Box sx={{ display: "flex", justifyContent: "flex-start", alignItems: "center", mb: 1 }}>
            <Tooltip title={layout === "grid" ? "Switch to compact list" : "Switch to card grid"}>
              <Button
                size="small"
                variant="outlined"
                startIcon={layout === "grid" ? <ViewModuleIcon /> : <ViewListIcon />}
                onClick={() => setLayout(layout === "grid" ? "list" : "grid")}
              >
                {layout === "grid" ? "Grid view" : "List view"}
              </Button>
            </Tooltip>
          </Box>
          <Box key={`${view}:${layout}`} sx={{ animation: `${slideUp} 250ms ease` }}>
            {view === "all" ? (
            <Box
              sx={{
                display: "flex",
                flexDirection: "column",
                gap: 1.5,
                maxWidth: layout === "list" ? 640 : "none",
              }}
            >
              {activePlatforms.filter((p) => (accountsByPlatform[p.id] ?? []).length > 0).length ===
              0 ? (
                <Typography color="text.secondary">
                  No saved accounts yet. Pick a platform on the left to add one.
                </Typography>
              ) : (
                activePlatforms.map((p) => {
                  const accs = accountsByPlatform[p.id] ?? [];
                  return accs.length > 0 ? renderPlatformGroup(p, accs) : null;
                })
              )}
            </Box>
          ) : (
            <Box sx={{ maxWidth: layout === "list" ? 640 : "none" }}>
              {renderPlatformGroup(
                activePlatforms[0] ?? {
                  id: view,
                  name: view,
                  account_count: 0,
                  detected: false,
                  enabled: true,
                },
                accountsByPlatform[view] ?? [],
                {
                  collapsible: false,
                  footer:
                    layout === "list" ? (
                      <>
                        {showImport && (
                          <ListItemButton
                            onClick={() => onImportCurrent(view)}
                            sx={{ color: "primary.main" }}
                          >
                            <ListItemIcon sx={{ minWidth: 44, color: "primary.main" }}>
                              <DownloadIcon />
                            </ListItemIcon>
                            <ListItemText primary="Import current login" />
                          </ListItemButton>
                        )}
                        <ListItemButton
                          onClick={() => onNewProfile(view)}
                          sx={{ color: "primary.main" }}
                        >
                          <ListItemIcon sx={{ minWidth: 44, color: "primary.main" }}>
                            <AddIcon />
                          </ListItemIcon>
                          <ListItemText primary="New profile" />
                        </ListItemButton>
                      </>
                    ) : (
                      <>
                        {showImport && (
                          <Card
                            variant="outlined"
                            sx={{ borderStyle: "dashed", borderColor: "primary.main", minHeight: 132 }}
                          >
                            <CardActionArea
                              onClick={() => onImportCurrent(view)}
                              sx={{
                                height: "100%",
                                p: 2,
                                display: "flex",
                                flexDirection: "column",
                                alignItems: "center",
                                justifyContent: "center",
                                gap: 1,
                                color: "primary.main",
                              }}
                            >
                              <DownloadIcon fontSize="large" />
                              <Typography variant="body2" color="primary" align="center">
                                Import current login
                              </Typography>
                            </CardActionArea>
                          </Card>
                        )}
                        <Card
                          variant="outlined"
                          sx={{ borderStyle: "dashed", borderColor: "primary.main", minHeight: 132 }}
                        >
                          <CardActionArea
                            onClick={() => onNewProfile(view)}
                            sx={{
                              height: "100%",
                              p: 2,
                              display: "flex",
                              flexDirection: "column",
                              alignItems: "center",
                              justifyContent: "center",
                              gap: 1,
                              color: "primary.main",
                            }}
                          >
                            <AddIcon fontSize="large" />
                            <Typography variant="body2" color="primary" align="center">
                              New profile
                            </Typography>
                          </CardActionArea>
                        </Card>
                      </>
                    ),
                },
              )}
            </Box>
          )}
          </Box>
        </Box>
      </Box>

      <NewProfileDialog
        open={!!newProfile}
        mode={newProfile?.mode ?? "fresh"}
        platformName={newProfileName}
        onClose={() => setNewProfile(null)}
        onSave={onSaveNew}
        onRename={onRenameExisting}
      />
      <AccountSettingsDialog
        open={!!editTarget}
        account={editTarget?.account ?? null}
        onClose={() => setEditTarget(null)}
        onSave={onSaveAccount}
        onDelete={onDeleteAccount}
      />
      <SettingsDialog
        open={settingsOpen}
        platforms={platforms}
        dataDir={dataDir}
        onChanged={refreshAll}
        onClose={() => setSettingsOpen(false)}
      />
      <UpdateNotifier />

      <Snackbar
        open={!!toast}
        autoHideDuration={3000}
        onClose={() => setToast(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "center" }}
      >
        {toast ? (
          <Alert severity={toast.sev} onClose={() => setToast(null)}>
            {toast.msg}
          </Alert>
        ) : undefined}
      </Snackbar>
    </Box>
  );
}
