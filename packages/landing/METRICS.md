# Search Visibility Metrics

How to measure whether the content strategy in `OPTIMIZED_CONTENT.md` is working. Record a row in the log below on the first week of each month; the baseline was taken 2026-07-04, before Google indexed the new pages.

## Automated on-page check

```sh
bun run build && bun run seo:audit
```

`scripts/seo-audit.mjs` validates every marketing page against the built output: title ≤65 chars, meta description 50–165 chars, canonical matching the sitemap (trailing slash), OG/twitter tags, exactly one `h1`, required JSON-LD types, and sitemap coverage. Run it after touching `MarketingLayout.astro`, `agent-pages.ts`, or any page head. It must pass before deploying.

## Tracked metrics

| Metric | Source | Target (2026-10, 90 days) |
|--------|--------|---------------------------|
| Position for `control claude code from phone` | Manual web search | Top 20 |
| Position for `run coding agent from iphone` | Manual web search | Top 20 |
| Position for `best ios app for ai coding agents` | Manual web search | Top 20 |
| Position for `zedra app` | Manual web search | Hold #1 |
| Indexed pages (`site:zedra.dev`) | Google | All sitemap URLs |
| GSC impressions / clicks per week | Search Console | Registered + trending up |
| Listicle / directory mentions | Manual (see outreach list) | ≥ 2 |
| App Store ratings count | App Store Connect | ≥ 10 |
| App Store search impressions | App Store Connect | Baseline recorded |
| GitHub stars | github.com/tanlethanh/zedra | Trend only, no target |

Position convention: record the ordinal of the first zedra.dev result on Google (US, logged out); `—` means absent from the first 3 pages.

## Log

| Date | claude code from phone | coding agent from iphone | best ios app | zedra app | Indexed | GSC clicks/wk | Mentions | AS ratings | Stars |
|------|-----------------------|--------------------------|--------------|-----------|---------|---------------|----------|------------|-------|
| 2026-07-04 | — | — | — | 1 | n/a (pages not deployed) | n/a (GSC not registered) | 0 | 0 | — |

Baseline notes (2026-07-04): intent SERPs are held by Anthropic's Remote Control docs, competitor listicles (tacticremote.com, getmoshi.app), and how-to guides. `zedra app`-shaped brand+category queries already resolve to zedra.dev #1. GSC registration and sitemap submission are the first actions after deploy — without them the GSC column stays empty.
