import { describe, it, expect } from "vitest";
import * as helpers from "./helpers";
import {
  formatNumber,
  formatCost,
  formatTokens,
  latencyColor,
  formatRelativeTime,
} from "../../lib/format";

describe("helpers re-exports", () => {
  it("re-exports formatNumber from lib/format", () => {
    expect(helpers.formatNumber).toBe(formatNumber);
  });

  it("re-exports formatCost from lib/format", () => {
    expect(helpers.formatCost).toBe(formatCost);
  });

  it("re-exports formatTokens from lib/format", () => {
    expect(helpers.formatTokens).toBe(formatTokens);
  });

  it("re-exports latencyColor from lib/format", () => {
    expect(helpers.latencyColor).toBe(latencyColor);
  });

  it("re-exports formatRelativeTime from lib/format", () => {
    expect(helpers.formatRelativeTime).toBe(formatRelativeTime);
  });

  it("re-exported formatNumber produces correct output", () => {
    expect(helpers.formatNumber(1500)).toBe("1.5k");
  });

  it("re-exported formatCost produces correct output", () => {
    expect(helpers.formatCost(1.5)).toBe("$1.50");
  });

  it("re-exported formatTokens produces correct output", () => {
    expect(helpers.formatTokens(1_500_000)).toBe("1.5M");
  });

  it("re-exported latencyColor produces correct output", () => {
    expect(helpers.latencyColor(100)).toBe("var(--color-success)");
  });
});
