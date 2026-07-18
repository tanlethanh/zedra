const siteUrl = "https://zedra.dev/";

export const organizationSchema = {
  "@context": "https://schema.org",
  "@type": "Organization",
  "@id": `${siteUrl}#organization`,
  name: "Zedra",
  url: siteUrl,
  logo: `${siteUrl}favicon-192.png`,
  sameAs: [
    "https://github.com/tanlethanh/zedra",
    "https://x.com/zedradev",
    "https://discord.gg/39MmkSS8sc",
    "https://apps.apple.com/app/zedra-code-from-anywhere/id6760534630",
    "https://play.google.com/store/apps/details?id=dev.zedra.app",
  ],
};

export const websiteSchema = {
  "@context": "https://schema.org",
  "@type": "WebSite",
  "@id": `${siteUrl}#website`,
  name: "Zedra",
  url: siteUrl,
  publisher: { "@id": `${siteUrl}#organization` },
  about: {
    "@type": "SoftwareApplication",
    name: "Zedra",
    applicationCategory: "DeveloperApplication",
    operatingSystem: "iOS, Android",
  },
};

export const baseSchemas = [organizationSchema, websiteSchema];
