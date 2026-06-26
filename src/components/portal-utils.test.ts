import { describe, it, expect } from "vitest";
import { centsToUSD, formatTime, computeProgressState } from "./portal-utils";

describe("centsToUSD", () => {
  it("converts 0 cents", () => {
    expect(centsToUSD(0)).toBe("$0.00");
  });

  it("converts round dollars", () => {
    expect(centsToUSD(500)).toBe("$5.00");
    expect(centsToUSD(1000)).toBe("$10.00");
  });

  it("converts cents with decimals", () => {
    expect(centsToUSD(599)).toBe("$5.99");
    expect(centsToUSD(1)).toBe("$0.01");
  });

  it("converts large amounts", () => {
    expect(centsToUSD(100000)).toBe("$1000.00");
    expect(centsToUSD(10000000)).toBe("$100000.00");
  });

  it("handles negative amounts (credits)", () => {
    expect(centsToUSD(-500)).toBe("$-5.00");
  });
});

describe("formatTime", () => {
  it("returns a non-empty string for valid ISO", () => {
    const result = formatTime("2026-06-26T12:00:00Z");
    expect(result).toBeTruthy();
    expect(typeof result).toBe("string");
  });

  it("handles Unix epoch", () => {
    const result = formatTime("1970-01-01T00:00:00Z");
    expect(result).toBeTruthy();
  });

  it("produces output containing the year for recent dates", () => {
    const result = formatTime("2026-06-26T12:00:00Z");
    expect(result).toContain("2026");
  });
});

describe("computeProgressState", () => {
  it("returns 0% pct when budget is null", () => {
    const result = computeProgressState(500, null);
    expect(result.pct).toBe(0);
    expect(result.isOverBudget).toBe(false);
    expect(result.isWarning).toBe(false);
    expect(result.barColor).toBe("var(--color-success)");
  });

  it("returns 0% pct with over-budget when budget is 0 (spent=budget)", () => {
    const result = computeProgressState(0, 0);
    expect(result.pct).toBe(0);
    expect(result.isOverBudget).toBe(true);
    expect(result.barColor).toBe("var(--color-danger)");
  });

  it("returns 0% pct with over-budget when spent exceeds zero budget", () => {
    const result = computeProgressState(5, 0);
    expect(result.pct).toBe(0);
    expect(result.isOverBudget).toBe(true);
  });

  it("calculates percentage for normal usage", () => {
    const result = computeProgressState(50, 100);
    expect(result.pct).toBe(50);
    expect(result.isOverBudget).toBe(false);
    expect(result.isWarning).toBe(false);
    expect(result.barColor).toBe("var(--color-success)");
  });

  it("caps percentage at 100", () => {
    const result = computeProgressState(200, 100);
    expect(result.pct).toBe(100);
    expect(result.isOverBudget).toBe(true);
    expect(result.barColor).toBe("var(--color-danger)");
  });

  it("detects over-budget when spent equals budget", () => {
    const result = computeProgressState(100, 100);
    expect(result.pct).toBe(100);
    expect(result.isOverBudget).toBe(true);
    expect(result.barColor).toBe("var(--color-danger)");
  });

  it("shows warning at 80%", () => {
    const result = computeProgressState(80, 100);
    expect(result.pct).toBe(80);
    expect(result.isOverBudget).toBe(false);
    expect(result.isWarning).toBe(true);
    expect(result.barColor).toBe("var(--color-warning)");
  });

  it("shows warning above 80% but below 100%", () => {
    const result = computeProgressState(90, 100);
    expect(result.pct).toBe(90);
    expect(result.isOverBudget).toBe(false);
    expect(result.isWarning).toBe(true);
    expect(result.barColor).toBe("var(--color-warning)");
  });

  it("does not show warning at 79%", () => {
    const result = computeProgressState(79, 100);
    expect(result.pct).toBe(79);
    expect(result.isWarning).toBe(false);
    expect(result.barColor).toBe("var(--color-success)");
  });

  it("over-budget takes precedence over warning", () => {
    const result = computeProgressState(150, 100);
    expect(result.pct).toBe(100);
    expect(result.isOverBudget).toBe(true);
    expect(result.isWarning).toBe(false);
    expect(result.barColor).toBe("var(--color-danger)");
  });

  it("zero spent with budget shows success", () => {
    const result = computeProgressState(0, 100);
    expect(result.pct).toBe(0);
    expect(result.isOverBudget).toBe(false);
    expect(result.isWarning).toBe(false);
    expect(result.barColor).toBe("var(--color-success)");
  });
});
