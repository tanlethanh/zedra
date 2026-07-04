# Optimized Content

How zedra.dev content is structured for search discoverability, how to verify it works, and what to do next. Findings and competitor facts below were verified on 2026-07-04; recheck them before acting on them.

## Goal

People searching for a way to control coding agents from a phone should find Zedra. Two problems stand in the way:

1. **Brand collision.** Google "zedra" is dominated by ZEDRA Group, a Swiss corporate/wealth firm with far higher domain authority. The bare brand query is not winnable short term.
2. **Missing intent content.** Users search by intent ("control claude code from phone", "run coding agent from iphone"), and competitors (Happy, Moshi, Omnara, Clauder, AgentsRoom) rank there while Zedra was absent.

The strategy: win intent queries with dedicated pages, and always pair the brand with the category ("Zedra — remote control for AI coding agents") so `zedra app` and `zedra coding` resolve to us even while bare `zedra` does not.

## Page inventory

| Page | Target queries | Notes |
|------|----------------|-------|
| `/` | brand + category, "remote control for AI coding agents" | Hero + agents, workspace, how-it-works, FAQ sections. `SoftwareApplication` schema with `sameAs`, `FAQPage` schema. |
| `/claude-code` | "run claude code from phone", "claude code iphone" | Generated from `src/agent-pages.ts` via `src/pages/[agent].astro`. |
| `/codex` | "codex cli from phone", "codex mobile" | Same template. |
| `/opencode` | "opencode mobile" | Same template. |
| `/compare` | "best ios app for ai coding agents", "zedra vs happy" | Honest table vs Happy, Omnara, SSH + tmux. |
| `/docs/*` | long-tail support queries | Starlight docs. |

All pages emit into `sitemap-index.xml` automatically via `@astrojs/sitemap`.

## Content rules

- Facts on marketing pages must match `/docs/installation`. When setup commands change, update `src/agent-pages.ts` and the homepage install tabs in the same change.
- Agent names must appear in crawlable text, not only in UI labels or SVGs.
- FAQ questions are written as literal search queries. Answers stay honest about limits (laptop sleep pauses the session; relays forward encrypted packets).
- The compare page states a "last checked" date. Recheck competitor pricing and agent support before editing it, and keep the tone generous — the page ranks and converts because it is credible.
- New agent page: add an entry to `src/agent-pages.ts`, link it from the homepage agents row and both footers.
- Follow the design idiom in `MarketingLayout.astro`: hairline rules, Lora serif headings, terminal-comment section labels, no filled cards, no white CTA pills.

## Verify

- **Google Search Console**: register `zedra.dev`, submit `https://zedra.dev/sitemap-index.xml`, watch query impressions weekly.
- **App Store Connect**: search impressions, conversion, and the search-terms report.
- **Monthly manual check** — Zedra should appear for:

```sh
# rerun these as web searches and note position
"control claude code from phone"
"run coding agent from iphone"
"best ios app for ai coding agents"
"zedra app"
```

- Structured data: paste page URLs into Google's Rich Results Test after meaningful changes.

## Backlog (highest leverage first)

1. **ASO.** App Store subtitle is "Code from anywhere" — zero search keywords. Add a keyword-bearing subtitle, fill the 100-char keyword field (claude, codex, opencode, terminal, agent, remote, git), and add an in-app rating prompt; the listing has 0 ratings and does not rank for "claude code remote".
2. **Listicle outreach.** The comparison articles that already rank (clauderc.com, sealos.io, tonydehnke.com, getmoshi.app) do not mention Zedra. Ask for inclusion.
3. **Launch spikes.** Show HN, Product Hunt, r/ClaudeAI — backlinks lift both Google rank and GitHub stars.
4. **Agent pages for Pi and Hermes** once their setup flows are stable enough to document.
