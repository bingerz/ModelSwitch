import { z } from "zod";

export const channelFormSchema = z.object({
  name: z.string().min(1, "Name is required"),
  provider: z.string().min(1, "Provider is required"),
  base_url: z.string().url("Must be a valid URL"),
  weight: z.number().int().min(0).default(1),
  priority: z.number().int().min(0).default(0),
});

export const virtualKeyFormSchema = z.object({
  name: z.string().min(1, "Name is required"),
  daily_budget_cents: z.number().int().min(0).nullable(),
  monthly_budget_cents: z.number().int().min(0).nullable(),
  rpm_limit: z.number().int().min(0).nullable(),
  tpm_limit: z.number().int().min(0).nullable(),
});

export type ChannelFormValues = z.infer<typeof channelFormSchema>;
export type VirtualKeyFormValues = z.infer<typeof virtualKeyFormSchema>;
