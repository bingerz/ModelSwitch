import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Ticket } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import { api } from "../lib/api";
import { useToast } from "./Toast";
import { formatCents } from "../lib/format";

export function RedemptionCodesPanel() {
  const { t } = useTranslation();
  const toast = useToast();
  const [showCreate, setShowCreate] = useState(false);
  const [newCredits, setNewCredits] = useState("");
  const [newExpiry, setNewExpiry] = useState("");
  const [redeemCode, setRedeemCode] = useState("");
  const [redeemUser, setRedeemUser] = useState("");
  const [creating, setCreating] = useState(false);
  const [redeeming, setRedeeming] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const { data: codes = [], isLoading: loading, refetch } = useQuery({
    queryKey: ["redemption-codes"],
    queryFn: () => api.redemptionCodes.list(),
    // Silently fail
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

  const handleCreate = async () => {
    const credits = parseFloat(newCredits);
    if (isNaN(credits) || credits <= 0) {
      toast.error(t("redemption.invalidCredits"));
      return;
    }
    setCreating(true);
    try {
      await api.redemptionCodes.create({
        credits_cents: Math.round(credits * 100),
        expires_at: newExpiry || null,
      });
      toast.success(t("redemption.created"));
      setNewCredits("");
      setNewExpiry("");
      setShowCreate(false);
      refresh();
    } catch {
      toast.error(t("redemption.createFailed"));
    } finally {
      setCreating(false);
    }
  };

  const handleRedeem = async () => {
    if (!redeemCode.trim()) return;
    setRedeeming(true);
    try {
      const result = await api.redemptionCodes.redeem(redeemCode.trim(), redeemUser || undefined);
      toast.success(t("redemption.redeemed", { credits: formatCents(result.credits_cents) }));
      setRedeemCode("");
      setRedeemUser("");
      refresh();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : t("redemption.redeemFailed"));
    } finally {
      setRedeeming(false);
    }
  };

  const handleDelete = async (code: string) => {
    if (confirmDelete !== code) {
      setConfirmDelete(code);
      return;
    }
    setConfirmDelete(null);
    try {
      await api.redemptionCodes.delete(code);
      toast.success(t("redemption.deleted"));
      refresh();
    } catch {
      toast.error(t("redemption.deleteFailed"));
    }
  };

  return (
    <section>
      <SectionHeader
        title={t("redemption.title")}
        icon={Ticket}
        onRefresh={refresh}
        refreshing={loading}
        action={
          <button className="btn btn-primary btn-sm" onClick={() => setShowCreate(!showCreate)}>
            {showCreate ? t("common.cancel") : t("redemption.create")}
          </button>
        }
      />

      {showCreate && (
        <div className="settings-section">
          <h3 className="settings-section-title">{t("redemption.createNew")}</h3>
          <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
            <div>
              <label className="settings-stat-label">{t("redemption.creditsAmount")}</label>
              <input
                type="number"
                className="settings-input"
                value={newCredits}
                onChange={(e) => setNewCredits(e.target.value)}
                placeholder="e.g. 10.00"
                step="0.01"
                min="0"
                style={{ width: "150px" }}
              />
            </div>
            <div>
              <label className="settings-stat-label">{t("redemption.expiresAt")}</label>
              <input
                type="datetime-local"
                className="settings-input"
                value={newExpiry}
                onChange={(e) => setNewExpiry(e.target.value)}
                style={{ width: "200px" }}
              />
            </div>
            <div style={{ display: "flex", alignItems: "flex-end" }}>
              <button className="btn btn-primary" onClick={handleCreate} disabled={creating}>
                {creating ? t("common.saving") : t("common.confirm")}
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="settings-section">
        <h3 className="settings-section-title">{t("redemption.redeemCode")}</h3>
        <div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
          <input
            type="text"
            className="settings-input"
            value={redeemCode}
            onChange={(e) => setRedeemCode(e.target.value)}
            placeholder={t("redemption.codePlaceholder")}
            style={{ width: "250px" }}
          />
          <input
            type="text"
            className="settings-input"
            value={redeemUser}
            onChange={(e) => setRedeemUser(e.target.value)}
            placeholder={t("redemption.userOptional")}
            style={{ width: "150px" }}
          />
          <button className="btn btn-sm" onClick={handleRedeem} disabled={redeeming || !redeemCode.trim()}>
            {redeeming ? t("common.loading") : t("redemption.redeem")}
          </button>
        </div>
      </div>

      {codes.length === 0 ? (
        <div className="empty-state">
          <div className="empty-state-icon">🎫</div>
          <div className="empty-state-title">{t("redemption.empty")}</div>
          <div className="empty-state-description">{t("redemption.emptyHint")}</div>
        </div>
      ) : (
        <div className="settings-table-wrapper">
          <table className="settings-table">
            <caption className="sr-only">{t("redemption.title")}</caption>
            <thead>
              <tr>
                <th scope="col">{t("redemption.code")}</th>
                <th scope="col">{t("redemption.credits")}</th>
                <th scope="col">{t("common.status")}</th>
                <th scope="col">{t("redemption.usedBy")}</th>
                <th scope="col">{t("redemption.usedAt")}</th>
                <th scope="col">{t("redemption.expires")}</th>
                <th scope="col">{t("common.actions")}</th>
              </tr>
            </thead>
            <tbody>
              {codes.map((c) => (
                <tr key={c.code}>
                  <th scope="row" className="mono" style={{ fontFamily: "var(--font-mono, monospace)" }}>{c.code}</th>
                  <td className="mono">{formatCents(c.credits_cents)}</td>
                  <td>
                    {c.used ? (
                      <span style={{ color: "var(--color-text-muted)" }}>{t("redemption.used")}</span>
                    ) : (
                      <span style={{ color: "var(--color-success)" }}>{t("redemption.available")}</span>
                    )}
                  </td>
                  <td className="mono">{c.used_by ?? "—"}</td>
                  <td className="mono">{c.used_at ? new Date(c.used_at).toLocaleString() : "—"}</td>
                  <td className="mono">{c.expires_at ? new Date(c.expires_at).toLocaleString() : "—"}</td>
                  <td>
                    <button
                      className="btn btn-sm btn-danger"
                      onClick={() => handleDelete(c.code)}
                      style={confirmDelete === c.code ? { background: "var(--color-danger)" } : {}}
                    >
                      {confirmDelete === c.code ? t("common.confirmQuestion") : t("common.delete")}
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
