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

export const issueTokenSchema = z.object({
  name: z.string().min(1, "Token name is required."),
  scope: z.enum(["write", "read"]),
  expires_in_days: z
    .union([z.coerce.number().int().min(1).max(365), z.literal("")])
    .optional(),
});
export type IssueTokenInput = z.infer<typeof issueTokenSchema>;
