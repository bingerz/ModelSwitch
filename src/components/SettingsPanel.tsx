import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Settings } from "lucide-react";
import { SectionHeader } from "./ui/SectionHeader";
import {
  api,
  type CacheStats,
  type GatewayInfo,
  type ProviderBudgetEntry,
} from "../lib/api";
import { GeneralSettings } from "./settings/GeneralSettings";
import { SecuritySection } from "./settings/SecuritySection";
import { GatewayInfoSection } from "./settings/GatewayInfoSection";
import { CacheManagementSection } from "./settings/CacheManagementSection";
import { ProviderBudgetsSection } from "./settings/ProviderBudgetsSection";
import { CompletionRatiosSection } from "./settings/CompletionRatiosSection";
import { BackupRestore } from "./settings/BackupRestore";

interface SettingsData {
  cache: CacheStats | null;
  info: GatewayInfo | null;
  budgets: ProviderBudgetEntry[];
  ratios: Record<string, number>;
}

export function SettingsPanel() {
  const { t } = useTranslation();

  const { data, isLoading: loading, refetch } = useQuery({
    queryKey: ["settings"],
    queryFn: async (): Promise<SettingsData> => {
      const [cache, info, budgetData] = await Promise.all([
        api.cacheStats().catch(() => null),
        api.gatewayInfo().catch(() => null),
        api.providerBudgets().catch(() => []),
      ]);
      let ratios: Record<string, number> = {};
      try {
        ratios = await api.completionRatios();
      } catch {
        // ratios not available yet
      }
      return { cache, info, budgets: budgetData, ratios };
    },
    refetchInterval: 10_000,
    // Silently fail — StatusBar shows gateway status
    retry: false,
  });

  const refresh = async () => {
    await refetch();
  };

  if (loading) return <div className="panel-loading">{t("settings.loading")}</div>;

  return (
    <section>
      <SectionHeader
        title={t("settings.title")}
        icon={Settings}
        onRefresh={refresh}
        refreshing={false}
      />

      <GeneralSettings onRefresh={refresh} />
      <SecuritySection />
      <GatewayInfoSection gatewayInfo={data?.info ?? null} onRefresh={refresh} />
      <CacheManagementSection
        cacheStats={data?.cache ?? null}
        onRefresh={refresh}
      />
      <ProviderBudgetsSection
        budgets={data?.budgets ?? []}
        onRefresh={refresh}
      />
      <CompletionRatiosSection
        initialRatios={data?.ratios ?? {}}
        onRefresh={refresh}
      />
      <BackupRestore />

      {/* Quick Links to Feature Panels */}
      <div className="settings-section">
        <h3 className="settings-section-title">
          {t("settings.advancedFeatures")}
        </h3>
        <p className="settings-hint">{t("settings.advancedFeaturesHint")}</p>
      </div>
    </section>
  );
}
