import { useState } from "react";
import { useTranslation } from "react-i18next";

const STORAGE_KEY = "modelswitch_onboarded";

type OnboardingStep = {
  readonly key: string;
  readonly tab: string;
};

const STEPS: readonly OnboardingStep[] = [
  { key: "channels", tab: "channels" },
  { key: "virtualKeys", tab: "virtualKeys" },
  { key: "playground", tab: "playground" },
] as const;

interface OnboardingBannerProps {
  /** Navigate to a tab id when a step link is clicked. */
  onNavigate: (tab: string) => void;
}

/**
 * First-run onboarding guide. Renders once for new users until dismissed.
 * Visibility is gated by localStorage so it survives reloads but doesn't
 * haunt returning users.
 */
export function OnboardingBanner({ onNavigate }: OnboardingBannerProps) {
  const { t } = useTranslation();
  const [dismissed, setDismissed] = useState(
    () => localStorage.getItem(STORAGE_KEY) === "true",
  );

  if (dismissed) return null;

  const dismiss = () => {
    localStorage.setItem(STORAGE_KEY, "true");
    setDismissed(true);
  };

  return (
    <section className="onboarding-banner" aria-label={t("onboarding.title")}>
      <div className="onboarding-header">
        <h2 className="onboarding-title">{t("onboarding.title")}</h2>
        <button
          type="button"
          className="onboarding-close"
          aria-label={t("common.dismiss")}
          onClick={dismiss}
        >
          &times;
        </button>
      </div>
      <p className="onboarding-subtitle">{t("onboarding.subtitle")}</p>
      <ol className="onboarding-steps">
        {STEPS.map((step, idx) => (
          <li key={step.key} className="onboarding-step">
            <span className="onboarding-step-num" aria-hidden="true">
              {idx + 1}
            </span>
            <button
              type="button"
              className="onboarding-step-link"
              onClick={() => onNavigate(step.tab)}
            >
              {t(`onboarding.steps.${step.key}`)}
            </button>
          </li>
        ))}
      </ol>
    </section>
  );
}
