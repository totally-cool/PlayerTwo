import { useEffect, useState, useCallback, useRef, type ReactNode } from "react";
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
  CircularProgress,
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
import ReplayIcon from "@mui/icons-material/Replay";
import RefreshIcon from "@mui/icons-material/Refresh";
import LightModeIcon from "@mui/icons-material/LightMode";
import DarkModeIcon from "@mui/icons-material/DarkMode";
import SettingsBrightnessIcon from "@mui/icons-material/SettingsBrightness";
import SettingsIcon from "@mui/icons-material/Settings";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, Account, AddResult, PlatformSummary, Settings } from "./api";
import { PlatformIcon, platformInfo, avatarColor, generatedAvatar } from "./platformIcons";
import { SettingsDialog, AccountSettingsDialog } from "./SettingsDialog";
import { UpdateNotifier } from "./UpdateNotifier";

const VIEW_KEY = "view";

/** Compact "3m ago" style relative time from a unix-seconds timestamp. */
function relativeTime(unixSecs: number): string {
  const diff = Math.max(0, Date.now() / 1000 - unixSecs);
  if (diff < 45) return "just now";
  const mins = Math.floor(diff / 60);
  if (mins < 60) return `${Math.max(1, mins)}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days}d ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  return `${Math.floor(months / 12)}y ago`;
}

/** Ordered, human-readable steps shown on a card while a switch is in flight. */
const SWITCH_STEPS = ["Closing…", "Swapping login…", "Launching…"] as const;

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
 * New-profile flow.
 *
 * Entry modes:
 * - "choose": ask whether to log out for a fresh login, or clone the account
 *   that's currently signed in. This is the default "New profile" path.
 * - "import": skip the question and go straight to naming the current login.
 *
 * Steps: "pick" (the choose question) → "wait" (fresh only: launcher is open,
 * waiting for the user to sign into a different account) → "name" (name & save).
 * If the captured account already exists, a banner offers to rename it instead.
 * Cancel is always available.
 */
function NewProfileDialog(props: {
  open: boolean;
  mode: "choose" | "import";
  platformName?: string;
  /** Whether an account is signed in now (i.e. there's a login to clone). */
  canClone: boolean;
  /** Whether the signed-in account is a live login NOT saved in the store, so a
   *  fresh sign-in would destroy it unless imported first. */
  currentUntracked: boolean;
  onClose: () => void;
  /** Trigger the log-out + open-launcher step for a fresh sign-in. */
  onStartFresh: () => void;
  /** Import (save) the currently signed-in login before it's cleared. */
  onImportCurrent: () => Promise<void>;
  onSave: (name: string) => Promise<AddResult>;
  onRename: (acc: Account) => void;
}) {
  type Step = "pick" | "confirm" | "wait" | "name";
  const [step, setStep] = useState<Step>("pick");
  const [flow, setFlow] = useState<"fresh" | "clone">("clone");
  const [name, setName] = useState("");
  const [exists, setExists] = useState<Account | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (props.open) {
      // "import" jumps straight to naming the current login; "choose" asks first.
      setStep(props.mode === "import" ? "name" : "pick");
      setFlow("clone");
      setName("");
      setExists(null);
      setBusy(false);
    }
  }, [props.open, props.mode]);

  const proceedFresh = () => {
    setFlow("fresh");
    props.onStartFresh();
    setStep("wait");
  };
  const chooseFresh = () => {
    // Guard the destructive path: if a live, unsaved login would be wiped, make
    // the user confirm (and offer to import it first).
    if (props.currentUntracked) {
      setFlow("fresh");
      setStep("confirm");
      return;
    }
    proceedFresh();
  };
  const importThenFresh = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await props.onImportCurrent();
      proceedFresh();
    } catch {
      // surfaced via parent toast
    } finally {
      setBusy(false);
    }
  };
  const chooseClone = () => {
    setFlow("clone");
    setStep("name");
  };

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

  const optionSx = {
    justifyContent: "flex-start",
    textAlign: "left" as const,
    textTransform: "none" as const,
    py: 1.25,
    px: 1.75,
  };

  return (
    <Dialog open={props.open} onClose={props.onClose} fullWidth maxWidth="sm">
      <DialogTitle>
        New profile
        {props.platformName ? ` — ${props.platformName}` : ""}
      </DialogTitle>
      <DialogContent>
        {step === "pick" ? (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5, mt: 1 }}>
            <DialogContentText>How do you want to create this profile?</DialogContentText>
            <Button variant="outlined" size="large" startIcon={<AddIcon />} sx={optionSx} onClick={chooseFresh}>
              <Box>
                <Typography sx={{ fontWeight: 600, color: "text.primary" }}>
                  Sign into a different account
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  Logs out of the current account and opens the launcher for a fresh sign-in.
                </Typography>
              </Box>
            </Button>
            <Button
              variant="outlined"
              size="large"
              startIcon={<DownloadIcon />}
              sx={optionSx}
              disabled={!props.canClone}
              onClick={chooseClone}
            >
              <Box>
                <Typography sx={{ fontWeight: 600, color: "text.primary" }}>
                  Clone the current login
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {props.canClone
                    ? "Saves the account you're signed into now — nothing is logged out."
                    : "No signed-in account was detected to clone."}
                </Typography>
              </Box>
            </Button>
          </Box>
        ) : step === "confirm" ? (
          <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5, mt: 1 }}>
            <Alert severity="warning">
              The account signed in right now isn’t saved in PlayerTwo. Logging out to sign into a
              different account will lose it unless you save it first.
            </Alert>
            <Button variant="contained" startIcon={<DownloadIcon />} sx={optionSx} disabled={busy} onClick={importThenFresh}>
              <Box>
                <Typography sx={{ fontWeight: 600 }}>Save it first, then log out</Typography>
                <Typography variant="caption" sx={{ opacity: 0.85 }}>
                  Imports the current login so you can switch back to it later.
                </Typography>
              </Box>
            </Button>
            <Button variant="outlined" color="error" sx={optionSx} disabled={busy} onClick={proceedFresh}>
              <Box>
                <Typography sx={{ fontWeight: 600 }}>Log out anyway</Typography>
                <Typography variant="caption" color="text.secondary">
                  Discards the current login and opens a fresh sign-in.
                </Typography>
              </Box>
            </Button>
          </Box>
        ) : step === "wait" ? (
          <DialogContentText>
            The launcher has opened. Sign into the <b>new</b> account, and once you’re fully
            logged in, click <b>“I’ve logged in”</b>.
          </DialogContentText>
        ) : (
          <>
            <DialogContentText sx={{ mb: 2 }}>
              {flow === "clone"
                ? "Save the account currently signed in — nothing is logged out."
                : "Name this profile to save the new login."}
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
        {step === "wait" ? (
          <Button variant="contained" onClick={() => setStep("name")}>
            I’ve logged in
          </Button>
        ) : step === "name" ? (
          <Button variant="contained" disabled={busy} onClick={save}>
            Save
          </Button>
        ) : null}
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
  // Per-card switch progress ("<platformId>:<accountId>" -> current step label)
  // and the last error for a card, driving an inline Retry action.
  const [switchStep, setSwitchStep] = useState<Record<string, string>>({});
  const [switchError, setSwitchError] = useState<Record<string, string>>({});
  // Cards whose switch reported success but whose launcher never signed in —
  // Epic discards a RememberMe token it no longer accepts without telling anyone,
  // so this is the only way the user finds out retrying is pointless.
  const [rejectedLogin, setRejectedLogin] = useState<Record<string, boolean>>({});
  const stepTimers = useRef<Record<string, number[]>>({});
  // Epic token freshness: accountId -> unix seconds the token was last saved.
  const [epicTokenAt, setEpicTokenAt] = useState<Record<string, number | null>>({});
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
    mode: "choose" | "import";
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
      // Epic rotates login tokens, so surface how fresh each saved token is.
      const epicAccs = accs["epic"] ?? [];
      if (epicAccs.length > 0) {
        const ages: Record<string, number | null> = {};
        await Promise.all(
          epicAccs.map(async (a) => {
            try {
              ages[a.id] = await api.epicTokenSavedAt(a.id);
            } catch {
              ages[a.id] = null;
            }
          }),
        );
        setEpicTokenAt(ages);
      }
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  }, []);

  // Refresh (renew) the active Epic account's rotating token, then re-read ages.
  const onRenewEpicToken = useCallback(
    async (accountId: string) => {
      try {
        await api.renewActiveTokens();
        const at = await api.epicTokenSavedAt(accountId);
        setEpicTokenAt((prev) => ({ ...prev, [accountId]: at }));
        // A freshly captured token is exactly what an expired one needed.
        setRejectedLogin((prev) => {
          const next = { ...prev };
          delete next[`epic:${accountId}`];
          return next;
        });
        setToast({ msg: "Login token refreshed", sev: "success" });
      } catch (e) {
        setToast({ msg: String(e), sev: "error" });
      }
    },
    [],
  );

  // Re-detect just the active account for one platform (light — no full reload).
  const refreshCurrent = useCallback(async (platformId: string) => {
    try {
      const id = await api.currentAccountId(platformId);
      setCurrentByPlatform((prev) => ({ ...prev, [platformId]: id }));
    } catch {
      /* leave the existing value in place */
    }
  }, []);

  // Re-detect the active account for every enabled platform (e.g. on refocus).
  const refreshCurrents = useCallback(async () => {
    await Promise.all(platforms.filter((p) => p.enabled).map((p) => refreshCurrent(p.id)));
  }, [platforms, refreshCurrent]);

  // Bumped on every switch so late poll callbacks from a superseded switch can't
  // clobber a newer one's result.
  const switchSeq = useRef(0);

  // The launcher applies the login change asynchronously (and may rewrite files
  // as it relaunches), so a single read right after a switch can still see the
  // old account. Poll a few times with backoff until the detection settles.
  const pollCurrentAfterSwitch = useCallback((platformId: string, expectedId: string) => {
    const seq = ++switchSeq.current;
    for (const ms of [1000, 2500, 5000, 9000]) {
      setTimeout(async () => {
        if (switchSeq.current !== seq) return; // superseded by a newer switch
        try {
          const id = await api.currentAccountId(platformId);
          // Ignore a transient "nothing detected" while the app is relaunching.
          if (id != null && switchSeq.current === seq) {
            setCurrentByPlatform((prev) => ({ ...prev, [platformId]: id }));
          }
        } catch {
          /* leave the optimistic value in place */
        }
      }, ms);
    }

    // Epic applies the login asynchronously and, when the saved token has
    // expired, throws it away without any error — the switch looks like a
    // success while the launcher sits on its sign-in screen. Once the polling
    // window has closed, ask the backend whether the sign-in was ever confirmed.
    if (platformId !== "epic") return;
    setTimeout(async () => {
      if (switchSeq.current !== seq) return;
      try {
        const rejected = await api.epicUnconfirmedSwitch();
        if (rejected !== expectedId || switchSeq.current !== seq) return;
        const key = `${platformId}:${expectedId}`;
        setRejectedLogin((prev) => ({ ...prev, [key]: true }));
        // Undo the optimistic highlight: this account is not actually active.
        setCurrentByPlatform((prev) =>
          prev[platformId] === expectedId ? { ...prev, [platformId]: null } : prev,
        );
        setToast({
          msg: "Epic rejected this saved login — it has expired. Sign in to Epic by hand once, then use Refresh login.",
          sev: "error",
        });
      } catch {
        /* detection is advisory; stay quiet if it fails */
      }
    }, 12000);
  }, []);

  useEffect(() => {
    // Keep the active account's rotating token (Epic) fresh, then load.
    api.renewActiveTokens().catch(() => {});
    refreshAll();
  }, [refreshAll]);

  // Re-check active accounts when the window regains focus — covers logins the
  // user completed in the launcher while PlayerTwo was in the background.
  useEffect(() => {
    const onFocus = () => {
      refreshCurrents();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refreshCurrents]);

  // Clear any pending switch-step timers when the app unmounts.
  useEffect(() => {
    const timers = stepTimers.current;
    return () => {
      Object.values(timers).forEach((list) => list.forEach((t) => clearTimeout(t)));
    };
  }, []);

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

  const clearStepTimers = useCallback((key: string) => {
    (stepTimers.current[key] ?? []).forEach((t) => clearTimeout(t));
    delete stepTimers.current[key];
  }, []);

  const onSwitch = async (platformId: string, acc: Account) => {
    const key = `${platformId}:${acc.id}`;
    // Ignore repeat clicks while this card is already switching.
    if (switchStep[key]) return;

    // Show staged progress on the clicked card. The backend switch is a single
    // blocking call, so we advance the labels on a timer as honest feedback and
    // settle when it returns.
    setSwitchError((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
    setRejectedLogin((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
    setSwitchStep((prev) => ({ ...prev, [key]: SWITCH_STEPS[0] }));
    clearStepTimers(key);
    stepTimers.current[key] = [
      window.setTimeout(
        () => setSwitchStep((p) => (p[key] ? { ...p, [key]: SWITCH_STEPS[1] } : p)),
        700,
      ),
      window.setTimeout(
        () => setSwitchStep((p) => (p[key] ? { ...p, [key]: SWITCH_STEPS[2] } : p)),
        1600,
      ),
    ];

    try {
      const out = await api.switchAccount(platformId, acc.id, settings?.auto_start ?? true);
      // Flip the active highlight immediately. Re-detecting the live account can
      // briefly lag (the launcher is relaunching and may rewrite files), so we
      // trust the switch result here and let refreshAll() reconcile.
      if (out.switched || out.already_active) {
        setCurrentByPlatform((prev) => ({ ...prev, [platformId]: acc.id }));
      }
      setToast({
        msg: out.already_active ? "Already active" : out.message || `Switched to ${acc.display_name}`,
        sev: out.already_active ? "info" : "success",
      });
      await refreshAll();
      // Keep checking for a few seconds — the launcher applies the change (and
      // may rewrite its login files) after relaunch, so the active account can
      // settle a moment later.
      pollCurrentAfterSwitch(platformId, acc.id);
      if (settings?.minimize_after_switch) {
        try {
          await getCurrentWindow().minimize();
        } catch {
          /* not running in a Tauri window */
        }
      }
    } catch (e) {
      // Surface the (possibly abort/timeout) error inline on the card with Retry,
      // as well as a toast.
      setSwitchError((prev) => ({ ...prev, [key]: String(e) }));
      setToast({ msg: String(e), sev: "error" });
    } finally {
      clearStepTimers(key);
      setSwitchStep((prev) => {
        const next = { ...prev };
        delete next[key];
        return next;
      });
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

  // "New profile": ask whether to log out for a fresh login or clone the
  // account signed in now (the chooser lives in NewProfileDialog).
  const onNewProfile = (platformId: string) => {
    setNewProfile({ platformId, mode: "choose" });
  };

  // Chosen "fresh login": log out and open the launcher for a fresh sign-in.
  const onStartFreshLogin = async () => {
    const platformId = newProfile?.platformId;
    if (!platformId) return;
    try {
      await api.prepareNewLogin(platformId);
    } catch (e) {
      setToast({ msg: String(e), sev: "error" });
    }
  };

  // Import the currently signed-in login before a destructive fresh-login clear,
  // so it isn't lost. Throws on failure so the dialog stays put.
  const onImportCurrentLogin = async () => {
    const platformId = newProfile?.platformId;
    if (!platformId) return;
    // Epic derives a name from its logs when none is given; others get a
    // placeholder the user can rename later.
    const name = platformId === "epic" ? "" : "Imported login";
    await api.addCurrentAccount(platformId, name);
    setToast({ msg: "Saved the current login", sev: "success" });
    await refreshAll();
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
  // A fresh sign-in for this platform would wipe a live login that isn't saved.
  const newProfilePlatform = newProfile?.platformId;
  const newProfileCurrent = newProfilePlatform ? currentByPlatform[newProfilePlatform] : null;
  const newProfileUntracked =
    !!newProfilePlatform &&
    newProfilePlatform !== "steam" &&
    newProfileCurrent != null &&
    !(accountsByPlatform[newProfilePlatform] ?? []).some((a) => a.id === newProfileCurrent);
  const detectedNames = platforms.filter((p) => p.detected).map((p) => p.name);

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
    const key = `${pid}:${acc.id}`;
    const step = switchStep[key];
    const error = switchError[key];
    const rejected = rejectedLogin[key];
    const busy = !!step;
    const isEpic = pid === "epic";
    const tokenAt = isEpic ? epicTokenAt[acc.id] : undefined;
    const secondaryText = error
      ? "Switch failed — tap retry"
      : busy
        ? step
        : rejected
          ? "Login expired — sign in to Epic once, then refresh"
          : isEpic && tokenAt
            ? `login saved ${relativeTime(tokenAt)}`
            : acc.note || undefined;
    return (
      <ListItem
        key={key}
        disablePadding
        secondaryAction={
          <Box sx={{ display: "flex", alignItems: "center" }}>
            {busy && <CircularProgress size={18} sx={{ mr: 0.5 }} />}
            {error && (
              <Tooltip title="Retry switch">
                <IconButton edge="end" size="small" color="warning" onClick={() => onSwitch(pid, acc)}>
                  <ReplayIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            )}
            {isEpic && isActive && !busy && !error && (
              <Tooltip title="Refresh saved login token">
                <IconButton edge="end" size="small" onClick={() => onRenewEpicToken(acc.id)}>
                  <RefreshIcon fontSize="small" />
                </IconButton>
              </Tooltip>
            )}
            <IconButton
              edge="end"
              size="small"
              onClick={() => setEditTarget({ platformId: pid, account: acc })}
            >
              <MoreVertIcon fontSize="small" />
            </IconButton>
          </Box>
        }
      >
        <ListItemButton selected={isActive} disabled={busy} onClick={() => onSwitch(pid, acc)}>
          <ListItemAvatar sx={{ minWidth: 44 }}>
            <Avatar
              src={acc.image ?? generatedAvatar(acc.id)}
              sx={{ width: 32, height: 32, fontSize: 14, bgcolor: av.bg, color: av.fg }}
            >
              {acc.display_name.charAt(0).toUpperCase()}
            </Avatar>
          </ListItemAvatar>
          <ListItemText
            primary={acc.display_name}
            secondary={
              secondaryText ? (
                <Typography
                  component="span"
                  variant="body2"
                  sx={{
                    color: error
                      ? "error.main"
                      : busy
                        ? "primary.main"
                        : rejected
                          ? "warning.main"
                          : "text.secondary",
                  }}
                >
                  {secondaryText}
                </Typography>
              ) : undefined
            }
          />
          {isActive && !busy && !error && (
            <CheckCircleIcon color="warning" fontSize="small" sx={{ mr: 1, opacity: 0.9 }} />
          )}
        </ListItemButton>
      </ListItem>
    );
  };

  const renderAccountCard = (pid: string, acc: Account) => {
    const isActive = currentByPlatform[pid] === acc.id;
    const av = avatarColor(acc.id);
    const key = `${pid}:${acc.id}`;
    const step = switchStep[key];
    const error = switchError[key];
    const rejected = rejectedLogin[key];
    const busy = !!step;
    const isEpic = pid === "epic";
    const tokenAt = isEpic ? epicTokenAt[acc.id] : undefined;
    return (
      <Card
        key={key}
        elevation={isActive ? 6 : 2}
        sx={{
          position: "relative",
          borderRadius: 2,
          border: isActive || rejected ? 2 : 0,
          borderStyle: "solid",
          borderColor: error ? "error.main" : rejected ? "warning.main" : "primary.main",
          transition: "box-shadow 150ms ease, transform 150ms ease",
          "&:hover": { boxShadow: 8, transform: "translateY(-3px)" },
        }}
      >
        <Box sx={{ position: "absolute", top: 6, left: 8, opacity: 0.7, zIndex: 1 }}>
          <PlatformIcon platformId={pid} size={16} brandColor />
        </Box>
        <CardActionArea
          disabled={busy}
          onClick={() => onSwitch(pid, acc)}
          sx={{ p: 2, pt: 3.5, display: "flex", flexDirection: "column", gap: 1 }}
        >
          <Avatar
            src={acc.image ?? generatedAvatar(acc.id)}
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
          {busy ? (
            <Box sx={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 0.5 }}>
              <CircularProgress size={18} />
              <Typography variant="caption" color="primary" noWrap>
                {step}
              </Typography>
            </Box>
          ) : error ? (
            <Typography variant="caption" color="error" noWrap>
              Switch failed
            </Typography>
          ) : rejected ? (
            <Tooltip title="Epic rejected this saved login. Sign in to Epic by hand once, then use Refresh login on the active card.">
              <Typography variant="caption" color="warning.main" noWrap>
                Login expired
              </Typography>
            </Tooltip>
          ) : isActive ? (
            <Chip size="small" color="warning" label="Active" />
          ) : isEpic && tokenAt ? (
            <Typography variant="caption" color="text.secondary" noWrap>
              saved {relativeTime(tokenAt)}
            </Typography>
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
        {error && (
          <Tooltip title="Retry switch">
            <IconButton
              size="small"
              color="warning"
              onClick={() => onSwitch(pid, acc)}
              sx={{ position: "absolute", bottom: 4, right: 4, zIndex: 1 }}
            >
              <ReplayIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        )}
        {isEpic && isActive && !busy && !error && (
          <Tooltip title="Refresh saved login token">
            <IconButton
              size="small"
              onClick={() => onRenewEpicToken(acc.id)}
              sx={{ position: "absolute", bottom: 4, left: 4, zIndex: 1 }}
            >
              <RefreshIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        )}
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
                  <Avatar src={activeAcc.image ?? generatedAvatar(activeAcc.id)}>
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
                      <ListItemButton
                        onClick={() => onNewProfile(view)}
                        sx={{ color: "primary.main" }}
                      >
                        <ListItemIcon sx={{ minWidth: 44, color: "primary.main" }}>
                          <AddIcon />
                        </ListItemIcon>
                        <ListItemText primary="New profile" />
                      </ListItemButton>
                    ) : (
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
        mode={newProfile?.mode ?? "choose"}
        platformName={newProfileName}
        canClone={newProfile ? currentByPlatform[newProfile.platformId] != null : false}
        currentUntracked={newProfileUntracked}
        onClose={() => setNewProfile(null)}
        onStartFresh={onStartFreshLogin}
        onImportCurrent={onImportCurrentLogin}
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
