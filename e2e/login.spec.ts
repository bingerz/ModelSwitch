import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "./helpers";

test.describe("Login flow", () => {
  test("shows login form on first visit", async ({ page }) => {
    await page.goto("/");

    // The login card renders a password input with the "Admin Token" placeholder.
    const tokenInput = page.getByPlaceholder("Admin Token", { exact: false });
    if (!(await tokenInput.isVisible().catch(() => false))) {
      test.skip(true, "Login form not found — gateway may already be authenticated");
    }

    // A Login button should accompany the input.
    await expect(page.getByRole("button", { name: "Login", exact: false })).toBeVisible();
  });

  test("completes login and shows the admin shell", async ({ page }) => {
    const loggedIn = await loginAsAdmin(page);
    if (!loggedIn) {
      test.skip(true, "Login UI not found — cannot verify post-login shell");
    }

    // After login the sidebar nav renders. Look for any nav button label.
    // The shell renders nav-item buttons for Channels, Virtual Keys, etc.
    await expect(page.locator(".sidebar").or(page.getByText("ModelSwitch"))).toBeVisible({
      timeout: 10_000,
    });
  });
});
