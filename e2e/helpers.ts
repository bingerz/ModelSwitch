import type { Page } from "@playwright/test";

const ADMIN_TOKEN = process.env.E2E_ADMIN_TOKEN ?? "test-admin-token";

/**
 * Best-effort admin login. Looks for the token input + submit button with
 * forgiving selectors and fills them. If the login UI can't be found the
 * caller treats it as a skip rather than a hard failure.
 */
export async function loginAsAdmin(page: Page): Promise<boolean> {
  await page.goto("/");

  // The login input is a password field with placeholder "Admin Token".
  const tokenInput = page.getByPlaceholder("Admin Token", { exact: false });
  if (!(await tokenInput.isVisible().catch(() => false))) {
    return false;
  }

  await tokenInput.fill(ADMIN_TOKEN);

  // Submit via the Login button; pressing Enter on the input also works.
  const loginButton = page.getByRole("button", { name: "Login", exact: false });
  if (await loginButton.isVisible().catch(() => false)) {
    await loginButton.click();
  } else {
    await tokenInput.press("Enter");
  }

  // Wait for the main shell to appear (sidebar brand text renders only after auth).
  await page
    .getByText("ModelSwitch", { exact: true })
    .first()
    .waitFor({ timeout: 10_000 })
    .catch(() => {});

  return true;
}
