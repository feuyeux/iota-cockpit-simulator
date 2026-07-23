import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

export type Theme = "dark" | "light";

const STORAGE_KEY = "cockpit:theme";
const LIGHT_CLASS = "theme-light";

interface ThemeValue {
  theme: Theme;
  setTheme: (theme: Theme) => void;
  toggleTheme: () => void;
}

const ThemeContext = createContext<ThemeValue | undefined>(undefined);

function initialTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  try {
    const stored = window.localStorage?.getItem(STORAGE_KEY);
    if (stored === "dark" || stored === "light") return stored;
  } catch {
    // Storage can be disabled in hardened browsers and test environments.
  }
  return "dark";
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, updateTheme] = useState<Theme>(initialTheme);
  useEffect(() => {
    document.documentElement.classList.toggle(LIGHT_CLASS, theme === "light");
    document.documentElement.style.colorScheme = theme;
  }, [theme]);
  const value = useMemo<ThemeValue>(() => ({
    theme,
    setTheme(next) {
      try {
        window.localStorage?.setItem(STORAGE_KEY, next);
      } catch {
        // Keep theme switching functional even when persistence is unavailable.
      }
      updateTheme(next);
    },
    toggleTheme() {
      const next = theme === "dark" ? "light" : "dark";
      try {
        window.localStorage?.setItem(STORAGE_KEY, next);
      } catch {
        // Keep theme switching functional even when persistence is unavailable.
      }
      updateTheme(next);
    }
  }), [theme]);

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeValue {
  const value = useContext(ThemeContext);
  if (!value) throw new Error("useTheme must be used inside ThemeProvider");
  return value;
}
