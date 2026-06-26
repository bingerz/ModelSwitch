import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    exclude: ["node_modules", "dist", "reference"],
    coverage: {
      provider: "v8",
      reporter: ["text", "text-summary", "lcov"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/**/*.test.{ts,tsx}",
        "src/**/*.spec.{ts,tsx}",
        "src/**/*.d.ts",
        "src/test-setup.ts",
        "src/**/types.ts",
        "src/**/mock-data.ts",
        "src/**/mock.ts",
      ],
      // Ratchet: raise as coverage improves. Baseline set just above current levels
      // to catch regressions without blocking incremental improvement.
      thresholds: {
        lines: 10,
        functions: 10,
        statements: 10,
        branches: 12,
      },
    },
  },
});
