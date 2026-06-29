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

/// Resolve a terminal's agent identity from its tracked metadata.
///
/// The foreground command is authoritative when present (shell integration sets
/// it via OSC 633). OSC 1 (`icon_name`) is only a fallback for terminals without
/// command lifecycle, and is ignored unless it itself names an agent — shells
/// commonly set OSC 1 to the cwd.
pub fn resolve_terminal_agent(
    command: Option<&str>,
    icon_name: Option<&str>,
) -> Option<&'static str> {
    // A present foreground command is authoritative even when it names no agent,
    // so running `vim` after an agent clears the identity instead of latching a
    // stale OSC 1. Only fall back to OSC 1 when there is no command lifecycle.
    match command.map(str::trim).filter(|c| !c.is_empty()) {
        Some(command) => detect_command(command),
        None => icon_name.and_then(detect_command),
    }
}

/// Resolve a command line to an agent slug, or `None` for a plain shell.
pub fn detect_command(raw: &str) -> Option<&'static str> {
    let low = raw.to_ascii_lowercase();
    let trimmed = low.trim();

    // Earliest match wins, so the agent named as the program beats one named in
    // a later flag value (`qwen --provider gemini` → qwen). Registry order
    // breaks position ties deterministically.
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

    #[test]
    fn detects_supported_agents() {
        assert_eq!(detect("amp"), Some("amp"));
        assert_eq!(detect("ampcode"), Some("amp"));
        assert_eq!(detect("claude"), Some("claude"));
        assert_eq!(detect("claude-code"), Some("claude"));
        assert_eq!(detect("cline"), Some("cline"));
        assert_eq!(detect("npx @openai/codex"), Some("codex"));
        assert_eq!(detect("github-copilot"), Some("copilot"));
        assert_eq!(detect("gh copilot suggest"), Some("copilot"));
        assert_eq!(detect("cursor-agent"), Some("cursor"));
        assert_eq!(detect("cursor agent"), Some("cursor"));
        assert_eq!(detect("gemini"), Some("gemini"));
        assert_eq!(detect("gemini-cli"), Some("gemini"));
        assert_eq!(detect("goose session"), Some("goose"));
        assert_eq!(detect("hermes"), Some("hermes"));
        assert_eq!(detect("hermes-agent"), Some("hermes"));
        assert_eq!(detect("junie"), Some("junie"));
        assert_eq!(detect("kilo"), Some("kilocode"));
        assert_eq!(detect("kilo-code"), Some("kilocode"));
        assert_eq!(detect("kilocode"), Some("kilocode"));
        assert_eq!(detect("open-claw"), Some("openclaw"));
        assert_eq!(detect("openclaw tui"), Some("openclaw"));
        assert_eq!(detect("open-code run"), Some("opencode"));
        assert_eq!(detect("opencode run"), Some("opencode"));
        assert_eq!(detect("open-hands"), Some("openhands"));
        assert_eq!(detect("openhands"), Some("openhands"));
        assert_eq!(detect("pi"), Some("pi"));
        assert_eq!(detect("npx @mariozechner/pi-coding-agent"), Some("pi"));
        assert_eq!(detect("qoder-cli"), Some("qoder"));
        assert_eq!(detect("qodercli"), Some("qoder"));
        assert_eq!(detect("qwen-code"), Some("qwen"));
        assert_eq!(detect("trae-agent"), Some("trae"));
        assert_eq!(detect("trae-cli interactive"), Some("trae"));
        assert_eq!(detect("zen-cli"), Some("zencoder"));
        assert_eq!(detect("zen cli"), Some("zencoder"));
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
