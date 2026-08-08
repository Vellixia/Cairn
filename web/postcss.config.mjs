// Tailwind v4 ships its own PostCSS plugin and handles vendor prefixing, so
// autoprefixer is gone rather than merely unused.
export default {
  plugins: { "@tailwindcss/postcss": {} },
};
