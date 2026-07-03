import { z } from "zod";

export const loginSchema = z.object({
  username: z.string().min(1, "Username is required."),
  password: z.string().min(1, "Password is required."),
});
export type LoginInput = z.infer<typeof loginSchema>;

export const setupSchema = z
  .object({
    username: z.string().min(1, "Username is required."),
    password: z.string().min(8, "Password must be at least 8 characters."),
    confirm: z.string().min(8, "Password must be at least 8 characters."),
  })
  .refine((v) => v.password === v.confirm, {
    path: ["confirm"],
    message: "Passwords do not match.",
  });
export type SetupInput = z.infer<typeof setupSchema>;

export const setupWizardSchema = z.object({
  username: z.string().min(1, "Username is required."),
  password: z.string().min(8, "Password must be at least 8 characters."),
  confirm: z.string().min(8, "Password must be at least 8 characters."),
  embed_provider: z.enum(["hashing", "local", "ollama", "openai"]),
  embed_model: z.string().optional(),
  embed_url: z.string().optional(),
  embed_api_key: z.string().optional(),
});
export type SetupWizardInput = z.infer<typeof setupWizardSchema>;

export const pairCodeSchema = z.object({
  name: z.string().min(1, "Device name is required."),
  ttl_minutes: z.coerce.number().int().min(1).max(60),
});
export type PairCodeInput = z.infer<typeof pairCodeSchema>;

export const issueTokenSchema = z.object({
  name: z.string().min(1, "Token name is required."),
  scope: z.enum(["admin", "write", "read"]),
  expires_in_days: z
    .union([z.coerce.number().int().min(1).max(365), z.literal("")])
    .optional(),
});
export type IssueTokenInput = z.infer<typeof issueTokenSchema>;

export const recallSchema = z.object({
  q: z.string().min(1, "Search query is required."),
});
export type RecallInput = z.infer<typeof recallSchema>;
