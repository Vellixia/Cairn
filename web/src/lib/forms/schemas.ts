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

export const rememberSchema = z.object({
  content: z.string().min(1, "Memory cannot be empty."),
});
export type RememberInput = z.infer<typeof rememberSchema>;

export const anchorSchema = z.object({
  goal: z.string().min(1, "Goal cannot be empty."),
});
export type AnchorInput = z.infer<typeof anchorSchema>;

export const checkpointSchema = z.object({
  label: z.string().max(120, "Label must be 120 characters or fewer.").optional(),
});
export type CheckpointInput = z.infer<typeof checkpointSchema>;

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

export const sanitizeSchema = z.object({
  text: z.string().min(1, "Text cannot be empty."),
});
export type SanitizeInput = z.infer<typeof sanitizeSchema>;

export const assembleSchema = z.object({
  paths: z
    .string()
    .min(1, "Add at least one path."),
  budget: z.coerce.number().int().min(100).max(1_000_000),
});
export type AssembleInput = z.infer<typeof assembleSchema>;

export const contextReadSchema = z.object({
  path: z.string().min(1, "Path is required."),
  mode: z.enum(["auto", "full", "signatures", "map"]).default("auto"),
});
export type ContextReadInput = z.infer<typeof contextReadSchema>;
