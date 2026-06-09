import {
  SiEpicgames,
  SiDiscord,
  SiGogdotcom,
  SiSteam,
  SiObsstudio,
  SiRiotgames,
  SiUbisoft,
  SiBattledotnet,
  SiRockstargames,
  SiPlaystation,
  SiNvidia,
  SiEa,
  SiOrigin,
  SiMeta,
} from "@icons-pack/react-simple-icons";
import VideogameAssetIcon from "@mui/icons-material/VideogameAsset";
import { useColorScheme } from "@mui/material/styles";

/** Xbox mark — simple-icons dropped it, so we ship our own (matches the
 *  simple-icons component signature: size / color / title). */
function XboxIcon({
  size = 24,
  color = "currentColor",
  title,
}: {
  size?: number;
  color?: string;
  title?: string;
}) {
  return (
    <svg role="img" viewBox="0 0 24 24" width={size} height={size} fill={color}>
      {title ? <title>{title}</title> : null}
      <path d="M4.102 21.033C6.211 22.881 8.977 24 12 24c3.026 0 5.789-1.119 7.902-2.967 1.877-1.912-4.316-8.709-7.902-11.417-3.582 2.708-9.779 9.505-7.898 11.417zm11.16-14.406c2.5 2.961 7.484 10.313 6.076 12.912C23.002 17.48 24 14.861 24 12.004c0-3.34-1.365-6.362-3.57-8.536 0 0-.027-.022-.082-.042-.063-.022-.152-.045-.281-.045-.592 0-1.985.434-4.805 3.246zM3.654 3.426c-.057.02-.082.041-.086.042C1.365 5.642 0 8.664 0 12.004c0 2.854.998 5.473 2.661 7.533-1.401-2.605 3.579-9.951 6.08-12.91-2.82-2.813-4.216-3.245-4.806-3.245-.131 0-.223.021-.281.046v-.002zM12 3.551S9.055 1.828 6.755 1.746c-.903-.033-1.454.295-1.521.339C7.379.646 9.659 0 11.984 0H12c2.334 0 4.605.646 6.766 2.085-.068-.046-.615-.372-1.52-.339C14.946 1.828 12 3.545 12 3.545v.006z" />
    </svg>
  );
}

/**
 * Brand icons for known platforms (via simple-icons), with their brand colors.
 * `colorDark` is used in dark mode when the brand color is too dark to see
 * (e.g. Epic's near-black, Steam's navy). Trademarks belong to their owners;
 * logos are used only to identify each platform. Unknown platforms fall back
 * to a generic icon.
 */
const ICONS: Record<
  string,
  {
    Icon: React.ComponentType<{ size?: number; color?: string; title?: string }>;
    color: string;
    colorDark?: string;
  }
> = {
  epic: { Icon: SiEpicgames, color: "#2F2D2E", colorDark: "#F2F2F2" },
  discord: { Icon: SiDiscord, color: "#5865F2" },
  "discord-canary": { Icon: SiDiscord, color: "#5865F2" },
  "discord-ptb": { Icon: SiDiscord, color: "#5865F2" },
  gog: { Icon: SiGogdotcom, color: "#86328A" },
  steam: { Icon: SiSteam, color: "#1B2838", colorDark: "#C7D5E0" },
  obs: { Icon: SiObsstudio, color: "#302E31", colorDark: "#E8E8E8" },
  riot: { Icon: SiRiotgames, color: "#D32936" },
  ubisoft: { Icon: SiUbisoft, color: "#1F1F1F", colorDark: "#E8E8E8" },
  battlenet: { Icon: SiBattledotnet, color: "#148EFF" },
  rockstar: { Icon: SiRockstargames, color: "#FCAF17" },
  "ps-remote-play": { Icon: SiPlaystation, color: "#003791", colorDark: "#7AB0FF" },
  "geforce-now": { Icon: SiNvidia, color: "#76B900" },
  ea: { Icon: SiEa, color: "#1F1F1F", colorDark: "#E8E8E8" },
  origin: { Icon: SiOrigin, color: "#F56C2D" },
  oculus: { Icon: SiMeta, color: "#0081FB" },
  xbox: { Icon: XboxIcon, color: "#107C10" },
};

export function PlatformIcon({
  platformId,
  size = 24,
  brandColor = false,
}: {
  platformId: string;
  size?: number;
  brandColor?: boolean;
}) {
  const { mode, systemMode } = useColorScheme();
  const isDark = (mode === "system" ? systemMode : mode) === "dark";

  const entry = ICONS[platformId];
  if (!entry) {
    // No brand icon: tint the placeholder controller from the palette.
    const color = brandColor ? avatarColor(platformId).bg : "currentColor";
    return <VideogameAssetIcon sx={{ fontSize: size, color }} />;
  }

  const { Icon, color, colorDark } = entry;
  const resolved = brandColor ? (isDark && colorDark ? colorDark : color) : "currentColor";
  return <Icon size={size} color={resolved} title={platformId} />;
}

/** The brand color for a platform (for accents/avatars), or a neutral default. */
export function platformColor(platformId: string): string {
  return ICONS[platformId]?.color ?? "#888";
}

/** Retro Arcade palette + black, white, and shades of grey. */
const AVATAR_PALETTE = [
  "#FFD700", // gold
  "#FF6347", // tomato
  "#8A2BE2", // blue-violet
  "#00BFFF", // deep sky blue
  "#FF69B4", // hot pink
  "#000000", // black
  "#FFFFFF", // white
  "#3A3A3A", // dark grey
  "#7A7A7A", // grey
  "#B0B0B0", // light grey
];

/** Pick a readable text color (black/white) for a given background hex. */
function contrastText(hex: string): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#000000" : "#FFFFFF";
}

/** Deterministic avatar colors from the palette above, derived from a seed
 *  (e.g. account id), with an auto-contrasting foreground. */
export function avatarColor(seed: string): { bg: string; fg: string } {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  const bg = AVATAR_PALETTE[h % AVATAR_PALETTE.length];
  return { bg, fg: contrastText(bg) };
}

/** Short, plain-language note about how switching works for each platform. */
const INFO: Record<string, string> = {
  steam:
    "Steam already remembers every account you've signed into — switching just changes which one logs in next. Each account needs to have been saved with 'Remember password' at least once.",
  epic:
    "We save Epic's login token for each account and swap it in on switch. You don't need to log out first; the launcher reopens already signed in.",
  gog: "Swaps GOG Galaxy's saved login (config + registry) so each profile signs in as a different user.",
  discord:
    "Swaps Discord's local session files, so each profile opens Discord logged into its own account.",
  "discord-canary": "Same as Discord, for the Canary build.",
  "discord-ptb": "Same as Discord, for the PTB build.",
  xbox:
    "Detected, but switching isn't supported here — Xbox sign-in is tied to your Windows account and managed by Windows itself.",
  riot: "Swaps the Riot Client's saved session so League / VALORANT / etc. sign in per profile.",
  ea: "Swaps EA Desktop's local login data between accounts.",
  origin: "Swaps Origin's local login data between accounts.",
  ubisoft: "Swaps Ubisoft Connect's saved session between accounts.",
  oculus: "Swaps the Oculus/Meta client's saved session between accounts.",
  "geforce-now": "Swaps GeForce NOW's saved session between accounts.",
  rockstar: "Swaps the Rockstar launcher's saved login between accounts.",
  jagex: "Swaps the Jagex launcher's saved session between accounts.",
};

export function platformInfo(platformId: string): string {
  return (
    INFO[platformId] ??
    "Switching saves and swaps this app's login files, so each profile signs in as its own account. Capture a profile while signed in, then switch any time the app is closed."
  );
}
