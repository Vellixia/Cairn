import type { Config } from "tailwindcss";

// Cairn brand tokens.
export default {
  content: ["./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#0B0F14",
        surface: "#12181F",
        surface2: "#1A2129",
        slate: "#8A94A6",
        offwhite: "#ECEFF4",
        ember: "#FB923C",
        teal: "#2DD4BF",
        line: "#222B35",
      },
      fontFamily: {
        sans: ["ui-sans-serif", "system-ui", "-apple-system", "Segoe UI", "Roboto", "Inter", "sans-serif"],
        mono: ["ui-monospace", "SFMono-Regular", "Menlo", "Consolas", "monospace"],
      },
    },
  },
  plugins: [],
} satisfies Config;
