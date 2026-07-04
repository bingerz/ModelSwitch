import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { z } from "zod";
import { useZodValidation } from "./useZodValidation";

const schema = z.object({
  name: z.string().min(1, "Name is required"),
  count: z.number().int().min(0),
});

describe("useZodValidation", () => {
  it("returns ok and parsed value on valid input", () => {
    const { result } = renderHook(() => useZodValidation(schema));

    let outcome: { ok: true } | { ok: false } = { ok: false };
    act(() => {
      outcome = result.current.validate({ name: "abc", count: 3 });
    });

    expect(outcome.ok).toBe(true);
    if (outcome.ok) {
      expect(outcome.value).toEqual({ name: "abc", count: 3 });
    }
    expect(result.current.errors).toEqual({});
  });

  it("returns field errors synchronously and updates state", () => {
    const { result } = renderHook(() => useZodValidation(schema));

    let outcome: { ok: true } | { ok: false } = { ok: false };
    act(() => {
      outcome = result.current.validate({ name: "", count: -1 });
    });

    expect(outcome.ok).toBe(false);
    if (!outcome.ok) {
      expect(outcome.errors.name).toBe("Name is required");
      expect(outcome.errors.count).toBeDefined();
    }
    expect(result.current.errors.name).toBe("Name is required");
  });

  it("clearError removes a single field", () => {
    const { result } = renderHook(() => useZodValidation(schema));
    act(() => {
      result.current.validate({ name: "", count: -1 });
    });
    act(() => {
      result.current.clearError("name");
    });
    expect(result.current.errors.name).toBeUndefined();
    expect(result.current.errors.count).toBeDefined();
  });

  it("clearAll empties every field", () => {
    const { result } = renderHook(() => useZodValidation(schema));
    act(() => {
      result.current.validate({ name: "", count: -1 });
    });
    act(() => {
      result.current.clearAll();
    });
    expect(result.current.errors).toEqual({});
  });
});
