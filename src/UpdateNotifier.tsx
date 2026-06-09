import { useEffect, useState } from "react";
import { Snackbar, Alert, Button } from "@mui/material";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/**
 * Checks for an update on mount and offers to install it. Silently does nothing
 * if the updater isn't configured (no endpoints/pubkey) or there's no update.
 */
export function UpdateNotifier() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    check()
      .then((u) => {
        if (u) setUpdate(u);
      })
      .catch(() => {
        /* updater not configured / offline — ignore */
      });
  }, []);

  const install = async () => {
    if (!update) return;
    setBusy(true);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setBusy(false);
    }
  };

  return (
    <Snackbar open={!!update} anchorOrigin={{ vertical: "bottom", horizontal: "left" }}>
      <Alert
        severity="info"
        action={
          <Button color="inherit" size="small" disabled={busy} onClick={install}>
            {busy ? "Installing…" : "Install & restart"}
          </Button>
        }
      >
        Update {update?.version} is available
      </Alert>
    </Snackbar>
  );
}
