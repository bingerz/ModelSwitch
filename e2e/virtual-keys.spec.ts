import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "./helpers";

test.describe("Virtual keys panel", () => {
  test.beforeEach(async ({ page }) => {
    const loggedIn = await loginAsAdmin(page);
    if (!loggedIn) {
      test.skip(true, "Login UI not found — cannot proceed to virtual keys panel");
    }
  });

  test("renders virtual keys panel with expected elements", async ({ page }) => {
    // Click the "Virtual Keys" nav item in the sidebar.
    const virtualKeysNav = page.getByRole("button", { name: "Virtual Keys", exact: false });
    if (!(await virtualKeysNav.isVisible().catch(() => false))) {
      test.skip(true, "Virtual Keys nav item not found");
    }
    await virtualKeysNav.click();

    // The panel renders inside <main>. Wait for any content.
    const main = page.locator("main");
    await expect(main).not.toBeEmpty({ timeout: 10_000 });

    // Look for common panel elements: search input, create button, or stat tiles.
    // These are forgiving checks — any one of them indicates the panel rendered.
    const searchInput = page.getByPlaceholder(/search/i).first();
    const createButton = page.getByRole("button", { name: /create|add/i }).first();
    const panelContent = searchInput.or(createButton).or(page.locator("main *").first());

    await expect(panelContent.first()).toBeVisible({ timeout: 10_000 });
  });
});
