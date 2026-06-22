import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  formatNumber,
  formatCost,
  formatCents,
  formatTokens,
  formatRelativeTime,
  latencyColor,
} from "./format";

describe("formatNumber", () => {
  it("returns rounded string for values below 1000", () => {
    // Arrange
    const input = 42;
    // Act
    const result = formatNumber(input);
    // Assert
    expect(result).toBe("42");
  });

  it("returns 0 as string for zero input", () => {
    expect(formatNumber(0)).toBe("0");
  });

  it("rounds decimals for values below 1000", () => {
    expect(formatNumber(42.7)).toBe("43");
  });

  it("formats thousands with k suffix", () => {
    expect(formatNumber(1000)).toBe("1.0k");
  });

  it("formats fractional thousands with one decimal", () => {
    expect(formatNumber(1500)).toBe("1.5k");
  });

  it("formats millions with M suffix", () => {
    expect(formatNumber(1_000_000)).toBe("1.0M");
  });

  it("formats fractional millions with one decimal", () => {
    expect(formatNumber(1_500_000)).toBe("1.5M");
  });

  it("handles negative values below 1000", () => {
    expect(formatNumber(-500)).toBe("-500");
  });

  it("handles negative thousands", () => {
    expect(formatNumber(-1500)).toBe("-1.5k");
  });

  it("handles negative millions", () => {
    expect(formatNumber(-1_500_000)).toBe("-1.5M");
  });

  it("treats 999 as plain number", () => {
    expect(formatNumber(999)).toBe("999");
  });
});

describe("formatCost", () => {
  it("formats zero as $0.00", () => {
    expect(formatCost(0)).toBe("$0.00");
  });

  it("formats small decimal value with two decimals", () => {
    expect(formatCost(1.5)).toBe("$1.50");
  });

  it("formats value under 1 with two decimals", () => {
    expect(formatCost(0.001)).toBe("$0.00");
  });

  it("formats thousands with k suffix", () => {
    expect(formatCost(1000.5)).toBe("$1.0k");
  });

  it("formats millions with M suffix", () => {
    expect(formatCost(1_000_000)).toBe("$1.0M");
  });

  it("formats typical small cost with two decimals", () => {
    expect(formatCost(12.345)).toBe("$12.35");
  });

  it("handles boundary just below 1000", () => {
    expect(formatCost(999.99)).toBe("$999.99");
  });
});

describe("formatCents", () => {
  it("formats zero cents as $0.00", () => {
    expect(formatCents(0)).toBe("$0.00");
  });

  it("formats 100 cents as $1.00", () => {
    expect(formatCents(100)).toBe("$1.00");
  });

  it("formats 599 cents as $5.99", () => {
    expect(formatCents(599)).toBe("$5.99");
  });

  it("formats 100000 cents as $1000.00", () => {
    expect(formatCents(100000)).toBe("$1000.00");
  });

  it("handles fractional cents by rounding to two decimals", () => {
    expect(formatCents(123.4)).toBe("$1.23");
  });
});

describe("formatTokens", () => {
  it("returns 0 string for zero input", () => {
    expect(formatTokens(0)).toBe("0");
  });

  it("returns plain number for values below 1000", () => {
    expect(formatTokens(500)).toBe("500");
  });

  it("formats thousands with k suffix", () => {
    expect(formatTokens(1000)).toBe("1.0k");
  });

  it("formats millions with M suffix", () => {
    expect(formatTokens(1_500_000)).toBe("1.5M");
  });

  it("matches formatNumber output (delegates internally)", () => {
    expect(formatTokens(23456)).toBe(formatNumber(23456));
  });
});

describe("formatRelativeTime", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-06-22T12:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("returns 'Just now' for timestamps less than a minute old", () => {
    // Arrange
    const now = new Date("2026-06-22T12:00:30Z"); // 30s ago
    // Act
    const result = formatRelativeTime(now);
    // Assert
    expect(result).toBe("Just now");
  });

  it("returns 'Just now' for exactly current time", () => {
    expect(formatRelativeTime(new Date("2026-06-22T12:00:00Z"))).toBe(
      "Just now"
    );
  });

  it("returns minutes for events under an hour old", () => {
    // 5 minutes ago
    const fiveMinAgo = new Date("2026-06-22T11:55:00Z");
    expect(formatRelativeTime(fiveMinAgo)).toBe("5m ago");
  });

  it("returns hours for events under a day old", () => {
    // 3 hours ago
    const threeHrAgo = new Date("2026-06-22T09:00:00Z");
    expect(formatRelativeTime(threeHrAgo)).toBe("3h ago");
  });

  it("returns days for events older than 24 hours", () => {
    // yesterday (exactly 24h ago)
    const yesterday = new Date("2026-06-21T12:00:00Z");
    expect(formatRelativeTime(yesterday)).toBe("1d ago");
  });

  it("accepts ISO date strings", () => {
    expect(formatRelativeTime("2026-06-22T11:55:00Z")).toBe("5m ago");
  });

  it("clamps future timestamps to 'Just now'", () => {
    // 10 seconds in the future
    const future = new Date("2026-06-22T12:00:10Z");
    expect(formatRelativeTime(future)).toBe("Just now");
  });
});

describe("latencyColor", () => {
  it("returns success color for latency below 500ms", () => {
    expect(latencyColor(499)).toBe("var(--color-success)");
  });

  it("returns success color for zero latency", () => {
    expect(latencyColor(0)).toBe("var(--color-success)");
  });

  it("returns warning color at 500ms boundary", () => {
    expect(latencyColor(500)).toBe("var(--color-warning)");
  });

  it("returns warning color for latency below 2000ms", () => {
    expect(latencyColor(1999)).toBe("var(--color-warning)");
  });

  it("returns orange color at 2000ms boundary", () => {
    expect(latencyColor(2000)).toBe("#f97316");
  });

  it("returns orange color for latency below 5000ms", () => {
    expect(latencyColor(4999)).toBe("#f97316");
  });

  it("returns danger color at 5000ms boundary", () => {
    expect(latencyColor(5000)).toBe("var(--color-danger)");
  });

  it("returns danger color for very high latency", () => {
    expect(latencyColor(10000)).toBe("var(--color-danger)");
  });
});
