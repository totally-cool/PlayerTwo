import { createTheme } from "@mui/material/styles";

/**
 * Basic Material UI v9 theme with light + dark color schemes.
 *
 * v9's theming is CSS-variable based, so both schemes are generated up front
 * and switched at runtime via the `useColorScheme` hook (see the toggle in
 * App.tsx). `colorSchemeSelector: "class"` lets us flip mode by class, which
 * pairs with <InitColorSchemeScript attribute="class"> in main.tsx.
 */
// Retro Arcade palette, reused across the app's accents.
const retro = {
  primary: { main: "#00BFFF" }, // deep sky blue
  secondary: { main: "#FF69B4" }, // hot pink
  info: { main: "#8A2BE2" }, // blue-violet
  warning: { main: "#FFD700" }, // gold
  error: { main: "#FF6347" }, // tomato
};

export const theme = createTheme({
  cssVariables: { colorSchemeSelector: "class" },
  colorSchemes: {
    light: {
      palette: {
        ...retro,
        // subtle cool tint instead of flat white/grey
        background: { default: "#F5F6FB", paper: "#FFFFFF" },
      },
    },
    dark: {
      palette: {
        ...retro,
        // subtle blue-violet tint instead of flat black/grey
        background: { default: "#101119", paper: "#181A24" },
      },
    },
  },
  shape: { borderRadius: 10 },
  typography: {
    button: { textTransform: "none", fontWeight: 600 },
  },
});
