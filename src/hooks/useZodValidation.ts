import { useState, useCallback } from "react";
import type { ZodSchema, ZodError } from "zod";

export type FieldErrors<T> = Partial<Record<keyof T, string>>;

export interface ZodValidationResult<T> {
  errors: FieldErrors<T>;
  /**
   * Validate `data` against the schema. Returns the parsed value on success
   * or `false` on failure (state is updated synchronously, and the errors
   * are also returned for use in the same tick — `errors` from the hook
   * closure is stale inside an event handler).
   */
  validate: (data: unknown) => { ok: true; value: T } | { ok: false; errors: FieldErrors<T> };
  clearError: (field: keyof T) => void;
  clearAll: () => void;
}

/**
 * Schema-based form validation powered by Zod.
 * Usage:
 *   const { validate } = useZodValidation(schema);
 *   const result = validate(formData);
 *   if (!result.ok) return;
 *   submit(result.value);
 */
export function useZodValidation<T>(schema: ZodSchema<T>): ZodValidationResult<T> {
  const [errors, setErrors] = useState<FieldErrors<T>>({});

  const validate = useCallback(
    (data: unknown): { ok: true; value: T } | { ok: false; errors: FieldErrors<T> } => {
      const result = schema.safeParse(data);
      if (result.success) {
        setErrors({});
        return { ok: true, value: result.data };
      }
      const fieldErrors: FieldErrors<T> = {};
      for (const issue of (result.error as ZodError).issues) {
        const field = issue.path[0] as keyof T;
        if (field && !fieldErrors[field]) {
          fieldErrors[field] = issue.message;
        }
      }
      setErrors(fieldErrors);
      return { ok: false, errors: fieldErrors };
    },
    [schema],
  );

  const clearError = useCallback((field: keyof T) => {
    setErrors((prev) => {
      if (!prev[field]) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
  }, []);

  const clearAll = useCallback(() => setErrors({}), []);

  return { errors, validate, clearError, clearAll };
}
