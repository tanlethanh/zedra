# Proof of done: selection survives scroll (Phase 1, re-anchor)

Sub-goal 01 of `zedra-select-scroll`. Fixes Phase 1 of tanlethanh/zedra#178.

## Acceptance criteria

| # | Criterion | Status |
|---|-----------|--------|
| 1 | `Terminal` stores the selection as absolute alacritty grid `Point`s (`Option<{anchor, focus}>`), not a viewport-relative `Range<usize>` | met |
| 2 | `TerminalSelectionDocument` retains each char's grid `(line, col)` and exposes `utf16_for_abs_point` + `abs_point_for_utf16` | met |
| 3 | The read API (`selection_range`) still yields a UTF-16 `Range<usize>`, now derived each frame by projecting `{anchor, focus}` onto the current document with edge-clamping | met |
| 4 | Scroll paths no longer invalidate the selection; next frame's projection reflects the new viewport | met |
| 5 | No `vendor/zed` change, no alacritty native `Selection`, no soft/hard-wrap copy-semantics change, IME/dictation branches untouched | met |
| 6 | Existing `includes_scrolled_scrollback_viewport` + soft-wrap tests still pass | met |
| 7 | Three new tests (survive-scroll / off-screen-clamp-and-resolve / soft-wrap-join) | met |
| 8 | Negative control: reverting the per-frame re-derive turns test (a) RED | met |

## Implementation summary

- `crates/zedra-terminal/src/terminal.rs`
  - New `TerminalSelection { anchor: Point, focus: Point }` (absolute alacritty grid points; `Point::line` is display-offset independent: `visible_row = line + display_offset`).
  - Field `selection_range: Option<Range<usize>>` -> `selection: Option<TerminalSelection>`.
  - `selection_range()` now DERIVES per call: builds a fresh document from current content (pixel origin irrelevant, `(0,0)`) and projects the stored points via `utf16_range_for_selection`.
  - `set_selection_range(range)` keeps its UTF-16 signature but re-anchors: `anchor = abs_point_for_utf16(range.start)`, `focus = abs_point_for_utf16(range.end - 1)`. Empty range clears (nothing to anchor).
  - `clear_selection_range` / `selection_active` retargeted to the new field.
- `crates/zedra-terminal/src/selection.rs`
  - `TerminalSelectionChar` gains `line: i32` (absolute) + `col: usize` (leading grid column; synthesized `\n` uses `grid_cols` as a virtual column so it sorts after real cells).
  - Public `utf16_for_abs_point(Point) -> Option<usize>`, `abs_point_for_utf16(usize) -> Option<Point>`, and `utf16_range_for_selection(anchor, focus) -> Range<usize>` (normalizes order, clamps a start whose point is below the viewport to `len`, an end above to `0`, so an off-screen anchor projects onto the visible remainder and re-resolves exactly on scroll-back).
- `input.rs`, `element.rs`, `view.rs`: **unchanged** for the selection path. The derive lives entirely in `terminal.rs`/`selection.rs`; the consumers still call `term.selection_range()` and get a UTF-16 range. IME/dictation/marked-text branches untouched.

## Confirmation run-table

| Command | Exit | Key output line |
|---------|------|-----------------|
| `cargo fmt --check -p zedra-terminal` | 0 | (no diff) |
| `cargo check --workspace` | 0 | `cargo build: 0 errors ... (443 crates)` |
| `cargo test -p zedra-terminal` | 0 | `178 passed (2 suites)` |
| `cargo test -- <3 new tests + includes_scrolled + omits_newline>` | 0 | `5 passed, 173 filtered out` |
| NC: `cargo test -- selection_survives_scroll_via_absolute_grid_points` (re-derive reverted) | 1 | `test result: FAILED. 0 passed; 1 failed` |

## Run detail

New tests (all in `crates/zedra-terminal/src/selection.rs` `mod tests`):

- `selection_survives_scroll_via_absolute_grid_points`, (a) select `MARK` at top of scrollback, `scroll(-1)`; derived range still covers `"MARK"`.
- `off_screen_anchor_clamps_and_resolves_exactly`, (b) select a 3-row span from `aaaa` through `MARK`; `scroll(-2)` pushes the anchor rows off-screen, derived range clamps to `start == 0` and covers `"cccc MARK"` (visible remainder); `scroll(2)` back re-resolves the EXACT original range and full text.
- `soft_wrapped_selection_keeps_wrap_join_after_scroll`, (c) `helloworld` soft-wrapped across two 5-col rows; selected range reads `"helloworld"` (no `\n`); after `scroll(-1)` the visible remainder is `"world"` with no stray `\n`; `scroll(1)` back restores `"helloworld"`.

Pre-existing guards re-run green: `includes_scrolled_scrollback_viewport`, `omits_newline_for_soft_wraps`.

## Reproduce

```
cd crates/zedra-terminal   # from the worktree root
cargo fmt --check -p zedra-terminal
cargo check --workspace
cargo test -p zedra-terminal
```

## Negative control (Rung 2), red on revert

Temporarily reverted the per-frame re-derive: cached the set-time viewport
`Range<usize>` in a scratch field and returned it from `selection_range()`
instead of re-projecting the absolute points. Test (a) then failed:

```
thread 'selection::tests::selection_survives_scroll_via_absolute_grid_points' panicked
  at crates/zedra-terminal/src/selection.rs:682:9:
assertion `left == right` failed
  left: Some("dddd")
 right: Some("MARK")
test result: FAILED. 0 passed; 1 failed; 0 ignored; 177 filtered out
```

The stale range, after `scroll(-1)`, now covers `"dddd"` (the text that slid
under those UTF-16 offsets) instead of the anchored `"MARK"`, exactly the
display-offset-keyed invalidation this change removes. The scratch field was
reverted (`git checkout`) and the suite re-run green afterward.
