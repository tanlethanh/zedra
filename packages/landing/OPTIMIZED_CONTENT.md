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

## SERP snapshot (2026-07-04)

- `"zedra" coding agent app` — **zedra.dev ranks #1**, GitHub #2, unikorn.vn #3. Brand + category pairing works.
- `control claude code from phone` — Zedra absent. Anthropic's official Remote Control docs rank #1, then how-to guides (builder.io, nxcode.io, zilliz.com, datasciencedojo) and competitors (agentsroom.dev, happy.engineering).
- `best ios app for ai coding agents` — Zedra absent. Ranking: Moshi's own listicle, Tactic Remote's listicle, codeagent-mobile.com. Every ranking listicle is competitor-published and omits Zedra.
- `run codex cli from phone` — Zedra absent. OpenAI's official "Codex in ChatGPT mobile" post plus Medium/dev.to how-tos and StarDesk/CC Pocket.
- Pattern: what ranks on intent queries is either official vendor docs or **how-to guides** — product pages alone rarely place. A `/docs` or blog-style "how to run Claude Code from your phone" guide is the likeliest organic wedge.

Competitor facts learned:

- **clauderc.com rebranded to tacticremote.com** (301 redirect). Their listicle compares Tactic Remote, Termius, Blink, Prompt 3, a-Shell — no Zedra. Contact: support page, X `@tacticremote`.
- **Moshi** (getmoshi.app) listicle compares Moshi, Blink, Termius, Prompt 3 — no Zedra. Contact: X `@odd_joel`, GitHub `rjyo`.
- Wider competitor set beyond the compare page: Moshi, Tactic Remote, CodeAgent Mobile, Codem, CC Pocket, StarDesk, CodeRemote, YoloCode, ClaudeCodeUI. Happy publishes its own alternatives page (`happy.engineering/docs/comparisons/alternatives/`) that ranks — same pattern our `/compare` follows.
- **App Store brand collision is worse than Google's**: an unrelated app literally named "Zedra" (id1541160685) and "Zedra Wallet" (zedra.app) both exist. Keyword-bearing subtitle + keyword field are the only way the listing surfaces.

## Backlog (highest leverage first)

1. **ASO.** App Store subtitle is "Code from anywhere" — zero search keywords. Add a keyword-bearing subtitle, fill the 100-char keyword field (claude, codex, opencode, terminal, agent, remote, git), and add an in-app rating prompt; the listing has 0 ratings and does not rank for "claude code remote". Urgent given the duplicate-name apps above.
2. **Directory listings (free backlinks, no gatekeeper).** Submit Zedra to AlternativeTo (its "Happy Coder Alternatives" page ranks) and launch on Product Hunt (its "Claude Code Remote Control alternatives" page ranks). Both are self-serve.
3. **Listicle outreach.** Ask for inclusion in the articles that already rank: tacticremote.com (`@tacticremote`), getmoshi.app (`@odd_joel`), zilliz.com, sealos.io, tonydehnke.com, agentsroom.dev.
4. **How-to guide content.** Intent SERPs reward guides over product pages. Add a docs/blog guide per agent ("How to run Claude Code from your phone") that the `/claude-code` page links to.
5. **Launch spikes.** Show HN, Product Hunt, r/ClaudeAI — backlinks lift both Google rank and GitHub stars.
6. **Agent pages for Pi and Hermes** once their setup flows are stable enough to document.
