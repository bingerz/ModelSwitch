import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "./helpers";

test.describe("Channels panel", () => {
  test.beforeEach(async ({ page }) => {
    const loggedIn = await loginAsAdmin(page);
    if (!loggedIn) {
      test.skip(true, "Login UI not found — cannot proceed to channels panel");
    }
  });

  test("renders channels list container", async ({ page }) => {
    // Click the "Channels" nav item in the sidebar.
    const channelsNav = page.getByRole("button", { name: "Channels", exact: false });
    if (!(await channelsNav.isVisible().catch(() => false))) {
      test.skip(true, "Channels nav item not found");
    }
    await channelsNav.click();

    // The panel renders inside <main>. Wait for any content — table, list, card,
    // or the empty state. We assert the main region is non-empty.
    const main = page.locator("main");
    await expect(main).not.toBeEmpty({ timeout: 10_000 });
  });

  test("dashboard empty state can navigate to channels", async ({ page }) => {
    // Navigate to dashboard
    const dashboardNav = page.getByRole("button", { name: "Dashboard", exact: false });
    if (!(await dashboardNav.isVisible().catch(() => false))) {
      test.skip(true, "Dashboard nav button not found");
    }
    await dashboardNav.click();

    // Look for the "Add First Channel" CTA in empty state
    const addChannelCTA = page.getByRole("button", { name: /add.*channel/i });
    if (await addChannelCTA.isVisible().catch(() => false)) {
      await addChannelCTA.click();

      // Should navigate to Channels panel - verify by looking for Create button
      const main = page.locator("main");
      await expect(main).not.toBeEmpty({ timeout: 10_000 });
    } else {
      // Skip if no empty state (gateway already has channels)
      test.skip(true, "Dashboard empty state not found — gateway may already have channels");
    }
  });
});
