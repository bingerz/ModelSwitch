import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Sun, Moon } from "lucide-react";
import { SectionHeader } from "../ui/SectionHeader";

type Theme = "light" | "dark";
type Lang = "en" | "zh";

function safeGetItem(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function safeSetItem(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore in test/SSR environments */
  }
}

function getInitialTheme(): Theme {
  const stored = safeGetItem("theme");
  if (stored === "light" || stored === "dark") return stored;
  try {
    return window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  } catch {
    return "light";
  }
}

function getInitialLang(): Lang {
  const stored = safeGetItem("lang");
  if (stored === "en" || stored === "zh") return stored;
  try {
    return navigator.language.startsWith("zh") ? "zh" : "en";
  } catch {
    return "en";
  }
}

/** Language and theme controls, redesigned as settings rows. */
export function AppearanceSettings() {
  const { t, i18n } = useTranslation();
  const [theme, setTheme] = useState<Theme>(getInitialTheme);
  const [lang, setLang] = useState<Lang>(getInitialLang);

  // Persist + apply theme whenever it changes.
  useEffect(() => {
    try {
      document.documentElement.setAttribute("data-theme", theme);
    } catch {
      /* ignore in test environments */
    }
    safeSetItem("theme", theme);
  }, [theme]);

  const handleLangChange = (next: Lang) => {
    if (next === lang) return;
    setLang(next);
    i18n.changeLanguage(next);
    safeSetItem("lang", next);
  };

  const handleThemeChange = (next: Theme) => {
    if (next === theme) return;
    setTheme(next);
  };

  return (
    <section>
      <SectionHeader title={t("settings.appearance")} icon={Sun} />

      <div className="settings-section">
        {/* Language */}
        <div className="appearance-row">
          <div className="appearance-row-label">
            <span className="appearance-row-title">
              {t("settings.language")}
            </span>
            <span className="appearance-row-hint">
              {t("settings.languageHint")}
            </span>
          </div>
          <select
            className="settings-select appearance-control"
            value={lang}
            onChange={(e) => handleLangChange(e.target.value as Lang)}
            aria-label={t("settings.language")}
          >
            <option value="en">{t("settings.english")}</option>
            <option value="zh">{t("settings.chinese")}</option>
          </select>
        </div>

        {/* Divider */}
        <div className="appearance-divider" />

        {/* Theme */}
        <div className="appearance-row">
          <div className="appearance-row-label">
            <span className="appearance-row-title">
              {t("settings.theme")}
            </span>
            <span className="appearance-row-hint">
              {t("settings.themeHint")}
            </span>
          </div>
          <div
            className="segmented-toggle appearance-control"
            role="group"
            aria-label={t("settings.theme")}
          >
            <button
              type="button"
              className={`segmented-option ${theme === "light" ? "active" : ""}`}
              onClick={() => handleThemeChange("light")}
              aria-pressed={theme === "light"}
            >
              <Sun size={14} className="icon-inline" />
              {t("settings.themeLight")}
            </button>
            <button
              type="button"
              className={`segmented-option ${theme === "dark" ? "active" : ""}`}
              onClick={() => handleThemeChange("dark")}
              aria-pressed={theme === "dark"}
            >
              <Moon size={14} className="icon-inline" />
              {t("settings.themeDark")}
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
