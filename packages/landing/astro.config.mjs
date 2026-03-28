import sitemap from "@astrojs/sitemap";
import vercel from "@astrojs/vercel";
// @ts-check
import { defineConfig } from "astro/config";

// https://astro.build/config
export default defineConfig({
  site: "https://zedra.dev",
  output: "static",
  adapter: vercel({ webAnalytics: { enabled: true } }),
  integrations: [sitemap()],
});
