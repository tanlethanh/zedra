//! Host-side agent identity detection.
//!
//! Resolves a foreground command line (and, as a fallback, an OSC 1 icon name)
//! to a registered agent slug. This is the single source of truth for terminal
//! agent identity: the host ships the resolved slug to clients, which render it
//! directly instead of re-running detection.
//!
//! Each actor declares its own aliases; detection just loops over the registry.
//! Matching is intentionally conservative — aliases match only at word
//! boundaries, and short names that double as common commands (`pi`, `hermes`)
//! match only as the entire command, so `cursor .` / `hermes build` / `pip
//! install` never latch an agent identity.

/// Resolve a terminal's agent identity. The foreground command (OSC 633) is
/// authoritative when present; OSC 1 is only a fallback and must itself name
/// an agent — shells commonly set it to the cwd.
pub fn resolve_terminal_agent(
    command: Option<&str>,
    icon_name: Option<&str>,
) -> Option<&'static str> {
    // A command naming no agent still clears identity (`vim` after an agent
    // must not latch a stale OSC 1).
    match command.map(str::trim).filter(|c| !c.is_empty()) {
        Some(command) => detect_command(command),
        None => icon_name.and_then(detect_command),
    }
}

/// Resolve a command line to an agent slug, or `None` for a plain shell.
pub fn detect_command(raw: &str) -> Option<&'static str> {
    let low = raw.to_ascii_lowercase();
    let trimmed = low.trim();

    // Earliest match wins (`qwen --provider gemini` → qwen); registry order
    // breaks position ties.
    let mut best: Option<(usize, &'static str)> = None;
    for actor in super::actors() {
        let at = if actor.detect_exact().iter().any(|needle| trimmed == *needle) {
            Some(0)
        } else {
            actor
                .detect_aliases()
                .iter()
                .filter_map(|needle| bounded_find(&low, needle))
                .min()
        };
        if let Some(at) = at {
            if best.is_none_or(|(best_at, _)| at < best_at) {
                best = Some((at, actor.slug()));
            }
        }
    }
    best.map(|(_, slug)| slug)
}

/// Byte offset of `needle` in `hay`, bounded by non-alphanumerics (or string
/// ends) on both sides — so `amp` matches `amp`/`npx amp` but not `sample`, and
/// `cursor-agent` matches itself but `cursor` never matches `cursor .`.
fn bounded_find(hay: &str, needle: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let at = from + rel;
        let before = hay[..at].chars().next_back();
        let after = hay[at + needle.len()..].chars().next();
        let bounded = before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !c.is_ascii_alphanumeric());
        if bounded {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::detect_command as detect;
    use super::resolve_terminal_agent as resolve;

    #[test]
    fn foreground_command_overrides_stale_icon() {
        // OSC 1 latched `codex`, then a non-agent command runs: command wins.
        assert_eq!(resolve(Some("vim"), Some("codex")), None);
        assert_eq!(resolve(Some("git status"), Some("codex")), None);
        // A present agent command beats a different stale icon.
        assert_eq!(resolve(Some("codex"), Some("claude")), Some("codex"));
    }

    #[test]
    fn icon_is_fallback_only_without_command_lifecycle() {
        // No command (cleared on command end): OSC 1 is the only signal.
        assert_eq!(resolve(None, Some("codex")), Some("codex"));
        // Blank command is treated as no command, not an authoritative shell.
        assert_eq!(resolve(Some("   "), Some("codex")), Some("codex"));
        // OSC 1 is ignored unless it itself names an agent (shells set cwd here).
        assert_eq!(resolve(None, Some("/home/me/project")), None);
        assert_eq!(resolve(Some("codex"), None), Some("codex"));
        assert_eq!(resolve(None, None), None);
    }

    // Walks the registry: every alias/exact token must resolve to its own
    // actor at word boundaries; exact-only tokens never latch inside a command.
    #[test]
    fn every_registered_alias_resolves_to_its_actor() {
        let mut aliases_checked = 0;
        for actor in crate::agent::actors() {
            let slug = actor.slug();
            for alias in actor.detect_aliases() {
                assert_eq!(detect(alias), Some(slug), "bare alias `{alias}`");
                assert_eq!(
                    detect(&format!("npx {alias}")),
                    Some(slug),
                    "embedded `{alias}`"
                );
                assert_eq!(
                    detect(&format!("{alias} --flag")),
                    Some(slug),
                    "trailing `{alias}`"
                );
                aliases_checked += 1;
            }
            for token in actor.detect_exact() {
                assert_eq!(detect(token), Some(slug), "exact token `{token}`");
                if !actor.detect_aliases().contains(token) {
                    assert_ne!(
                        detect(&format!("{token} build")),
                        Some(slug),
                        "exact-only token `{token}` must not match inside a longer command"
                    );
                }
            }
        }
        assert!(
            aliases_checked > 0,
            "registry declared no detection aliases"
        );
        assert_eq!(detect("zsh"), None);
    }

    #[test]
    fn detection_avoids_partial_tokens() {
        assert_eq!(detect("sample"), None);
        assert_eq!(detect("hermes build"), None);
        assert_eq!(detect("openhanded"), None);
        assert_eq!(detect("cursor ."), None);
        assert_eq!(detect("cline auth -p openai"), Some("cline"));
        assert_eq!(detect("qwen --provider openai"), Some("qwen"));
        assert_eq!(detect("qwen --provider gemini"), Some("qwen"));
        assert_eq!(detect("pip install pytest"), None);
    }
}
