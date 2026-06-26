import { describe, it, expect } from "vitest";
import {
  dollarsToCents,
  centsToDollars,
  formatDate,
  CENTS_PER_DOLLAR,
} from "./types";

describe("dollarsToCents", () => {
  it("returns null for empty string", () => {
    expect(dollarsToCents("")).toBeNull();
  });

  it("returns null for whitespace-only string", () => {
    expect(dollarsToCents("   ")).toBeNull();
  });

  it("converts whole dollar value", () => {
    expect(dollarsToCents("10")).toBe(1000);
  });

  it("converts decimal dollar value", () => {
    expect(dollarsToCents("10.50")).toBe(1050);
  });

  it("converts zero dollars to zero cents", () => {
    expect(dollarsToCents("0")).toBe(0);
  });

  it("returns null for negative value", () => {
    expect(dollarsToCents("-5")).toBeNull();
  });

  it("returns null for non-numeric string", () => {
    expect(dollarsToCents("abc")).toBeNull();
  });

  it("returns null for Infinity", () => {
    expect(dollarsToCents("Infinity")).toBeNull();
  });

  it("rounds fractional cents using Math.round", () => {
    // 10.999 * 100 = 1099.9 -> Math.round -> 1100
    expect(dollarsToCents("10.999")).toBe(1100);
  });

  it("trims surrounding whitespace before converting", () => {
    expect(dollarsToCents("  5  ")).toBe(500);
  });

  it("uses CENTS_PER_DOLLAR constant consistently", () => {
    expect(dollarsToCents("1")).toBe(CENTS_PER_DOLLAR);
  });
});

describe("centsToDollars", () => {
  it("returns empty string for null", () => {
    expect(centsToDollars(null)).toBe("");
  });

  it("returns whole number string for even dollar amount", () => {
    expect(centsToDollars(1000)).toBe("10");
  });

  it("returns decimal string for non-even amount", () => {
    expect(centsToDollars(1050)).toBe("10.50");
  });

  it("returns '0' for zero cents", () => {
    expect(centsToDollars(0)).toBe("0");
  });

  it("roundtrips with dollarsToCents for whole dollar", () => {
    expect(centsToDollars(dollarsToCents("42")!)).toBe("42");
  });

  it("roundtrips with dollarsToCents for decimal dollar", () => {
    expect(centsToDollars(dollarsToCents("42.55")!)).toBe("42.55");
  });
});

describe("formatDate", () => {
  it("returns a non-empty localized string for a valid ISO date", () => {
    const result = formatDate("2024-01-15T00:00:00Z");
    expect(typeof result).toBe("string");
    expect(result.length).toBeGreaterThan(0);
  });

  it("does not throw and returns a string for invalid date input", () => {
    // `new Date('not-a-date')` produces an Invalid Date object; calling
    // toLocaleDateString on it does NOT throw — it returns 'Invalid Date'.
    // The try/catch in formatDate therefore never fires for this input;
    // verify the function's actual contract: it does not throw and returns
    // a non-empty string.
    const result = formatDate("not-a-date");
    expect(typeof result).toBe("string");
    expect(result.length).toBeGreaterThan(0);
  });

  it("does not throw for empty string input", () => {
    expect(() => formatDate("")).not.toThrow();
  });
});
