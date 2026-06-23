import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatRelativeTime } from "./helpers";

export interface LiveBadgeProps {
  lastUpdated: Date;
}

/**
 * Pulsing green "Live" indicator with relative time. Re-renders once per
 * second so the "updated Xs ago" label stays current without forcing the
 * parent to poll faster.
 */
export function LiveBadge({ lastUpdated }: LiveBadgeProps) {
  const { t } = useTranslation();
  const [, setTick] = useState(0);

  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <span className="dsh-live-badge" title={lastUpdated.toLocaleTimeString()}>
      <span className="dsh-live-dot" aria-hidden="true" />
      <span className="dsh-live-label">{t("common.live")}</span>
      <span className="dsh-live-relative">{formatRelativeTime(lastUpdated)}</span>
    </span>
  );
}
