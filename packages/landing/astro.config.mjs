// @ts-check
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";
import vercel from "@astrojs/vercel";
import { defineConfig } from "astro/config";

// Open external links in a new tab; internal (/...) links stay in place.
function rehypeExternalBlank() {
  const visit = (node) => {
    if (
      node.type === "element" &&
      node.tagName === "a" &&
      typeof node.properties?.href === "string" &&
      /^https?:\/\//.test(node.properties.href)
    ) {
      node.properties.target = "_blank";
      node.properties.rel = ["noopener", "noreferrer"];
    }
    node.children?.forEach(visit);
  };
  return (tree) => visit(tree);
}

// https://astro.build/config
export default defineConfig({
  site: "https://zedra.dev",
  output: "static",
  adapter: vercel({ webAnalytics: { enabled: true } }),
  markdown: {
    rehypePlugins: [rehypeExternalBlank],
  },
  integrations: [
    starlight({
      title: "Zedra",
      description:
        "Documentation for Zedra — a mobile remote code editor for your desktop workspace.",
      customCss: ["./src/styles/docs.css"],
      // Code blocks: reuse the app's One Dark syntax palette (see
      // crates/zedra/src/editor/syntax_theme.rs), no window frame, and the
      // site's near-black surface instead of One Dark's default background.
      expressiveCode: {
        themes: ["one-dark-pro"],
        defaultProps: { frame: "none" },
        styleOverrides: {
          codeBackground: "#0d0d0e",
          borderColor: "var(--sl-color-hairline)",
          // EC adds the 1px border width on top, so 7px renders as an 8px
          // outer radius — matching the tab container below.
          borderRadius: "7px",
        },
      },
      // Match the dark/mono landing page; no light theme.
      components: {
        SiteTitle: "./src/components/docs/SiteTitle.astro",
        Footer: "./src/components/docs/Footer.astro",
        // Dark-only site: drop the theme toggle entirely.
        ThemeSelect: "./src/components/docs/ThemeSelect.astro",
        // Open GitHub/Discord social links in a new tab.
        SocialIcons: "./src/components/docs/SocialIcons.astro",
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/tanlethanh/zedra",
        },
        {
          icon: "discord",
          label: "Discord",
          href: "https://discord.gg/39MmkSS8sc",
        },
      ],
      sidebar: [
        { label: "Why Zedra?", slug: "docs" },
        { label: "Installation", slug: "docs/installation" },
        { label: "Security & architecture", slug: "docs/security" },
        { label: "Troubleshooting", slug: "docs/troubleshooting" },
        {
          label: "References",
          items: [
            { label: "Telemetry", slug: "docs/telemetry" },
            { label: "Privacy", link: "/privacy", attrs: { target: "_self" } },
            { label: "Support", link: "/support", attrs: { target: "_self" } },
          ],
        },
      ],
    }),
    sitemap(),
  ],
});
