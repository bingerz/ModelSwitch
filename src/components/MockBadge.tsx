import { isMockMode } from "../lib/mock";

export function MockBadge() {
  if (!isMockMode()) return null;
  return (
    <div
      style={{
        position: "fixed",
        bottom: 16,
        right: 16,
        background: "#dc2626",
        color: "white",
        fontSize: 11,
        fontWeight: 600,
        padding: "4px 10px",
        borderRadius: 999,
        zIndex: 9999,
        pointerEvents: "none",
        letterSpacing: 0.3,
        fontFamily: "ui-monospace, monospace",
      }}
    >
      ● DEMO MODE
    </div>
  );
}
