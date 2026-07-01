import { defineConfig, devices } from "@playwright/test";

// Prerequisite: start gateway (cd src-tauri && cargo run --bin modelswitch-cli -- serve) and Vite (pnpm dev) before running E2E
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: process.env.E2E_BASE_URL ?? "http://localhost:1420",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  outputDir: "test-results",
});
