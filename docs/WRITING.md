# Writing Style

Write Zedra docs in a practical, direct, task-first style. Assume the reader is
scanning while trying to do something.

## Voice

- Start with the action or goal. Put background after the common path.
- Use short sentences in present tense.
- Address the reader as `you` in user-facing docs.
- Be specific. Name the command, file, setting, platform, or state.
- State limitations directly. Pair them with the next step or workaround when
  there is one.
- Avoid promotional, apologetic, or defensive wording.

Avoid filler and hedging:

- `just`
- `simply`
- `easily`
- `seamless`
- `powerful`
- `robust`
- `best-in-class`

## Structure

- Lead with the default workflow.
- Move setup before customization.
- Put rare edge cases near the end.
- Use headings that name the thing the reader is looking for.
- Prefer short paragraphs over dense bullet lists.
- Keep lists parallel: action, action, action.

## Examples

Use complete examples. Do not show fragments that require guessing.

Use `sh` fences for terminal commands:

```sh
scripts/generate-assets.sh
```

Use backticks for paths, commands, settings, and keybindings:

- `crates/zedra/assets/icons/<slug>.svg`
- `bun run icons:gen`
- `format_on_save`
- `Cmd+Shift+P`

## Good

To add an icon, create `crates/zedra/assets/icons/<slug>.svg` with
`currentColor`. Builds generate the iOS imageset and Android drawable.

Run this when you need to inspect the generated iOS assets locally:

```sh
scripts/generate-assets.sh
```

## Avoid

Zedra provides a powerful and seamless icon pipeline. Simply add your icon and
the system will easily take care of everything for you.

## Links

Use relative links inside docs:

- Same directory: `[Manual Test](./MANUAL_TEST.md)`
- Parent directory: `[AGENTS.md](../AGENTS.md)`
- Anchor: `[Icon Assets](../AGENTS.md#icon-assets)`
