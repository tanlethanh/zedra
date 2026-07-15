# ASO Plan

App Store Optimization plan for Zedra on iOS (primary) and Google Play (secondary). Builds on the ASO findings in `OPTIMIZED_CONTENT.md` (backlog item 1) and the tracking in `METRICS.md`. Product facts verified 2026-07-09; ASO best practices verified against Apple Developer docs, AppTweak, and AppFollow 2025-2026 guidance.

## Current state (2026-07-09)

| Field | iOS | Google Play |
|-------|-----|-------------|
| Name | `Zedra - Code from anywhere` (26/30) | `Zedra` |
| Subtitle / short desc | `Remote control AI workspace` (27/30) | (none set) |
| Keyword field | (unknown / not optimized) | n/a (Play indexes description) |
| Category | Developer Tools | (unconfirmed — likely Developer Tools) |
| Ratings (US) | 0 | 0 |
| Ratings (VN) | 2 × 5.0 | 0 |
| Screenshots | none checked in; 6 device captures exist in `packages/landing/src/assets/demo/` | none |
| App preview video | none | none |
| Rating prompt | **not implemented** — no `SKStoreReviewController` in the codebase | n/a |
| Description | (in App Store Connect, not in repo) | (in Play Console, not in repo) |

**Three biggest drags:** (1) zero US ratings, (2) name/subtitle miss every high-intent term (claude, codex, agent, terminal), (3) no screenshots or preview video checked in.

## Priority matrix

| # | Lever | Effort | Impact | Why |
|---|-------|--------|--------|-----|
| 1 | In-app rating prompt | Low | High | 0 US ratings is the single biggest ranking + conversion drag. `SKStoreReviewController`, 3 prompts/365 days max. |
| 2 | iOS name + subtitle + keyword field | Low | High | Title is the highest-weight indexed field. Current listing misses claude/codex/agent/terminal entirely. Competitors pack these into the name (App Review precedent: "Omnara: Claude & Codex Mobile", "Happy: Codex & Claude Code App"). |
| 3 | App Store screenshots (first 3) | Medium | High | First 3 screenshots appear in search results — the impression that decides tap-through. Source captures already exist. |
| 4 | App preview video (iOS) | Medium | Medium | 15-30s muted autoplay in search results. Demo footage exists (`packages/landing/public/zedra-demo-17-06.mp4`). |
| 5 | Google Play full listing | Medium | Medium | Play indexes the long description (4000 chars) — a direct ranking field iOS doesn't have. Currently underinvested (version 0.5, no short desc). |
| 6 | Apple Search Ads (brand + keyword) | Medium | Medium | ASA keyword data reveals which dev-tool terms convert; fold winners into organic metadata. Brand campaign protects against the "Zedra" collision apps. |
| 7 | Localization (en-US → ja, de) | High | Medium | Developer-tool demand clusters in JP/DE. Both markets prefer fully localized screenshots. Do after en-US is proven. |

## iOS App Store

### Name (30 chars max) — highest ranking weight

Current: `Zedra - Code from anywhere` (26) — indexed: zedra, code, anywhere.

Proposed: `Zedra: AI Coding Agent Remote` (29)
- Adds: coding, agent, remote (three high-intent terms absent today).
- Brand + category pattern matches competitor precedent that passes App Review.
- Character count: `Zedra: AI Coding Agent Remote` = 29/30.

Alternate if App Review pushes back on length: `Zedra: Coding Agent Remote` (25).

### Subtitle (30 chars max) — second highest weight, shows in search results

Current: `Remote control AI workspace` (27) — indexed: remote, control, AI, workspace.

Proposed: `Claude Code, Codex & terminal` (29)
- Adds: claude, code, codex, terminal — the four highest-intent agent/tool terms.
- Do not repeat words from the name (Apple ignores repeats; wastes space).
- Character count: `Claude Code, Codex & terminal` = 29/30.

Alternate: `Claude Code & Codex terminal` (27).

### Keyword field (100 chars max, hidden, comma-separated, no spaces between terms)

Rule: never repeat words already in name or subtitle. Apple ignores duplicates.

Proposed (96/100):
```
opencode,gemini,git,ssh,cli,ide,editor,mobile,dev,shell,tmux,mosh,diff,vibe,pair,phone,workspace
```

What this covers (semantic clusters, per 2026 algorithm shift toward intent diversity):
- **Agent names not in subtitle:** opencode, gemini
- **Workspace nouns:** git, diff, editor, ide, cli, shell, workspace
- **Connection/platform:** ssh, tmux, mosh, pair, phone, mobile
- **Dev intent:** dev, vibe

Avoid in keyword field: "app", "code" (in subtitle), "claude" (in subtitle), "codex" (in subtitle), "terminal" (in subtitle), "remote" (in name), "agent" (in name), plurals of included singulars.

### Promotional text (170 chars, updatable anytime without new version)

Not indexed for ranking, but visible on the product page. Update for launches, feature drops, seasonal hooks. Keep the first sentence compelling — it's the first thing scrollers read after the screenshot.

Draft:
```
Free and open source. Control Claude Code, Codex, OpenCode, or any CLI agent from your phone over an encrypted P2P tunnel. Terminal, editor, git diff, and markdown — no VPN.
```
(168/170)

### Description (not indexed on iOS — write for conversion, not keywords)

Structure:
1. One-line hook (first line visible before "read more").
2. What it is (2-3 sentences).
3. Feature bullets (terminal, agents, file browser, editor, git, markdown).
4. How it works (pair by QR → encrypted tunnel → your machine stays the source of truth).
5. Agents supported (Claude Code, Codex, OpenCode, Pi, Hermes + any CLI).
6. Free & open source (MIT). No cloud hosting of your code.
7. FAQ inline (mirror the landing page FAQ: laptop sleep, relay encryption, subscription).

### Screenshots (up to 10; first 3 appear in search results)

Source captures ready in `packages/landing/src/assets/demo/`. Compose into 1290×2796 (iPhone 6.7") frames with text overlays.

Recommended order (first 3 = search impression):

| # | Source capture | Text overlay | Purpose |
|---|----------------|--------------|---------|
| 1 | `remote-terminal.png` | "Run AI agents from your phone" | Core value prop — the "why" |
| 2 | `manage-agents.png` | "Approve prompts & track usage" | The unique workflow (approvals + usage) |
| 3 | `git.png` | "Review git diffs before commit" | The "trust" feature — review agent changes |
| 4 | `code-editor.png` | "Read code with syntax highlighting" | Editor capability |
| 5 | `markdown.png` | "Rendered markdown & Mermaid" | Docs/plans reading |
| 6 | `file-explorer-search.png` | "Browse files & search repos" | File management |

Rules:
- Include dark-mode frames (Zedra is dark by default — the App Store background is dark, so this is natural).
- Text overlays: minimal, high-contrast, legible at search-result thumbnail size.
- First 3 must communicate value in <2 seconds — they're the search-result impression.
- Localize text overlays per locale (start en-US).

### App preview video (up to 3; first auto-plays muted in search, 15-30s)

Source: `packages/landing/public/zedra-demo-17-06.mp4` (existing hero footage).

Plan:
1. Cut a 20-30s clip: connect (QR) → terminal with agent running → approval prompt → git diff → done.
2. First 3 seconds must be visually obvious (muted autoplay) — show the terminal with an agent working.
3. Real device-captured UI only (Apple requires this — no commercial-style animation).
4. Poster frame: the terminal screenshot.
5. Localize text overlays for en-US first.

### Category

Current: `Developer Tools` (primary). This is correct — it's where Termius, Blink, Textastic, and similar dev tools sit. Keep it. Consider adding `Productivity` as secondary if App Store Connect allows it (broadens browse reach without misclassifying).

### Icon

Current icon (`AppIcon-1024.png`) is already in place. Icon is legible and branded. No change needed unless A/B testing shows a tap-through problem. If testing later: keep it simple, high-contrast, recognizable at 40×40px (search-result size).

## Google Play

Google Play indexes the **long description** (4000 chars) — a ranking field iOS doesn't have. This is the biggest ASO difference and the place to invest writing effort.

### Title (30 chars)

Proposed: `Zedra: AI Coding Agent Remote` (29) — same as iOS for brand consistency.

### Short description (80 chars) — indexed, on-page conversion

Draft:
```
Remote control for Claude Code, Codex & any CLI agent. Terminal, editor, git diff.
```
(80/80)

### Long description (4000 chars) — indexed, target ~2-3% keyword density

Structure (write naturally, don't stuff):
1. Hook line (repeat core value with key terms).
2. What Zedra is (2-3 paragraphs, natural keyword use: "coding agent", "remote terminal", "code editor", "git diff", "Claude Code", "Codex", "OpenCode").
3. Feature sections (terminal, agents, file browser, editor, git, markdown) — each a short paragraph with natural keyword repetition.
4. How it works (P2P, encrypted, no VPN, QR pairing).
5. Supported agents (list them — these are search terms).
6. Free & open source (MIT).
7. FAQ.

Target terms to weave naturally (not stuffed): coding agent, AI coding agent, remote terminal, code editor, mobile dev, git diff, markdown, Claude Code, Codex, OpenCode, Gemini CLI, SSH, shell, pair programming, vibe coding, developer tools, iOS, Android.

### Screenshots (up to 8)

Same source captures as iOS. Google Play screenshots are hidden in general search but appear on the app page — more functional/detailed style works. Reuse the iOS frames; optionally add 2 more detailed workflow shots.

### Feature graphic (1024×500)

Create a feature graphic for the Play Store listing header: Zedra logo + tagline "Remote control for AI coding agents" + a faint device mockup. This is the Play Store equivalent of the App Store hero — don't skip it.

### Category

Confirm `Developer Tools` in Play Console. If the listing isn't set up fully (version 0.5 suggests it's behind iOS), prioritize completing the Play Console listing.

## Reviews & ratings strategy

**The #1 ASO lever for Zedra right now.** Zero US ratings means the listing is invisible in social-proof terms even if it ranks.

### iOS: `SKStoreReviewController`

- **Not yet implemented** in the codebase (verified 2026-07-09 — no StoreKit review calls found).
- Implement: prompt after a high-value moment — a successful session connect, or after the user approves an agent prompt and the agent completes a task.
- Apple caps at **3 prompts per 365-day period** per device. Don't waste prompts on app launch or low-engagement moments.
- Use the standard system prompt (do not build a custom review dialog — Apple rejects those).
- Gate the prompt: only show if the user has had at least one successful session (don't prompt users who haven't paired yet).

### Google Play: in-app review API

- Use the Google Play In-App Review API (`com.google.android.play:review`).
- Same gating: after a successful session.
- Play doesn't have the same hard cap; still don't over-prompt.

### Review responding

- Respond to every review via App Store Connect / Play Console.
- Reviewer is notified and can update their rating. A thoughtful response to a 1-2 star review can flip it to 4-5.
- Only the latest review + response per user displays — re-respond when a user updates.

### Benchmark

- **500+ ratings at 4.5+ stars** is where conversion materially improves.
- Zedra's 90-day target (per `METRICS.md`): ≥ 10 US ratings. Realistic for an early-stage open-source app.
- Track rating sentiment weekly by country and version.

## Localization

Phase 1 (now): en-US only. Get the listing proven first.

Phase 2 (after en-US stabilizes, ~8-12 weeks): prioritize by developer-tool demand:
1. **Japanese** — large iOS developer market; prefers fully localized screenshots.
2. **German** — strong developer-tool market; prefers localized screenshots.
3. Then: Simplified Chinese, Korean, French, Spanish.

Rules:
- **Don't translate keywords — research them per market.** Search intent differs by language.
- Localize: name, subtitle, keyword field (iOS — it's per-locale), description, promotional text, screenshots (text overlays), app preview video (text overlays).
- Google Play: use Custom Store Listings (CSLs) for region-specific full listings.

## Apple Search Ads (ASA)

### Brand protection campaign

Run a minimal brand campaign on "Zedra" to ensure the listing appears above the collision apps ("Zedra" id1541160685, "Zedra Wallet"). Without it, a brand search may surface the wrong app.

### Keyword discovery

Run ASA on high-intent dev-tool terms ("claude code", "codex", "remote terminal", "code editor", "ssh client") and read the conversion data. Terms that convert well in ASA should be folded into organic metadata (name/subtitle/keyword field) — this is the ASA → ASO feedback loop.

### Budget

Start small. ASA for a free app with no IAP has no direct ROAS — the goal is discovery data and install volume (which lifts organic ranking). Treat it as market research spend.

## Implementation sequence

Do these in order. Don't change keywords and creatives simultaneously — you can't isolate impact. Allow 3-4 weeks between significant metadata updates for ranking stabilization.

### Sprint 1 (week 1) — metadata + ratings
1. **Update iOS name + subtitle + keyword field** in App Store Connect (requires a new app version submission).
   - Name: `Zedra: AI Coding Agent Remote`
   - Subtitle: `Claude Code, Codex & terminal`
   - Keyword field: `opencode,gemini,git,ssh,cli,ide,editor,mobile,dev,shell,tmux,mosh,diff,vibe,pair,phone,workspace`
2. **Implement `SKStoreReviewController`** in the iOS app — prompt after a successful session, gated on at least one completed agent task. See `ios/Zedra/Presentations.swift` for the native bridge pattern; add a new `@_cdecl` entry point and a Rust call from the session-success path.
3. **Set promotional text** (can update without a new version).
4. **Write the iOS description** (conversion-focused, not keyword-stuffed).

### Sprint 2 (weeks 2-3) — screenshots + video
5. **Compose 6 App Store screenshots** from `packages/landing/src/assets/demo/` captures (1290×2796 frames, text overlays, order per table above).
6. **Cut a 20-30s app preview video** from `zedra-demo-17-06.mp4` (muted-autoplay-friendly first 3 seconds).
7. Upload both to App Store Connect.

### Sprint 3 (weeks 3-4) — Google Play
8. **Complete the Play Console listing**: title, short description (80 chars), long description (4000 chars with natural keyword density), feature graphic, screenshots.
9. **Implement Google Play In-App Review API** — same gating as iOS.
10. Confirm category = Developer Tools.

### Sprint 4 (weeks 4-6) — ASA + measure
11. **Launch ASA brand campaign** ("Zedra") + a small keyword discovery campaign.
12. **Record metrics** in `METRICS.md` (first week of month): App Store search impressions, US ratings count, keyword positions.

### Ongoing
- Update promotional text monthly (feature drops, no version needed).
- Respond to every review within 48 hours.
- Re-research keyword positions every 4 weeks; adjust keyword field only with a new version submission.
- Evaluate ASO impact over quarters, not days. Strategic results show in 2-3 month cycles.

## Measurement

Track in `METRICS.md` (first week of each month):

| Metric | Source | Target (90 days) |
|--------|--------|-------------------|
| App Store ratings (US) | App Store Connect | ≥ 10 |
| App Store search impressions | App Store Connect | Baseline → trending up |
| Position: `claude code` (App Store) | App Store search | Top 50 |
| Position: `coding agent` (App Store) | App Store search | Top 50 |
| Position: `remote terminal` (App Store) | App Store search | Top 50 |
| Position: `zedra app` (App Store) | App Store search | #1 |
| Google Play installs (organic) | Play Console | Trending up |
| Product page conversion rate | App Store Connect | ≥ 40% (baseline first) |
| Review sentiment (by country, by version) | App Store Connect / Play Console | No dips below 4.0 avg |

Tools:
- **App Store Connect** — built-in analytics (limited but free).
- **Google Play Console** — more granular than App Store Connect.
- **ASO intelligence tool** (AppTweak / AppFollow / Sensor Tower) — for keyword rank history, competitor tracking, sentiment analysis, visibility score. Pick one when budget allows.

## Risks & notes

- **Brand collision** is severe on the App Store (two unrelated "Zedra" apps exist). The keyword-bearing name + subtitle is the mitigation. Do not drop the brand from the name — always `Zedra: ...`.
- **App Review risk** for keyword-packing the name: competitors ("Omnara: Claude & Codex Mobile", "Happy: Codex & Claude Code App") pass App Review with this pattern. If rejected, fall back to the shorter alternate name.
- **June 2025 algorithm shift**: Apple now favors intent diversity in top results over a single intent type. The keyword strategy above covers multiple semantic clusters (agent names, workspace nouns, connection terms) — this aligns with the shift.
- **Apple LLM-generated app tags** (iOS 26): Apple auto-generates tags from your metadata. Keep metadata clean and specific — vague descriptions produce vague tags.
- **Don't change keywords and screenshots in the same update** — you can't isolate which one moved the needle. Stagger by at least one indexation cycle (3-4 weeks).
