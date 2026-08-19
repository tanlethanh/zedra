// OpenCode 2.0 ships as its own `opencode2` binary alongside v1, so it is a
// separate detect-only actor rather than a launch variant of `opencode`.
simple_actor!(
    OpenCode2Actor,
    "opencode2",
    "OpenCode v2",
    "opencode",
    ["opencode2"],
    ["opencode2"],
    "--auto"
);
