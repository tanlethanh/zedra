# Performance instrumentation (`--perf`)

Read only when `--perf` passed — different shape from correctness debugging.

Here per-frame logs expected, but only when structured for aggregation,
never as raw dump.

- Log one line per frame/event with **aggregation key** (e.g.
  screen/component name) and **duration or count**, not full state.
- After repro window, aggregate: parse captured lines
  (`ios-log.sh query --since <window start>`), group by key, compute
  min/max/p50/p95/count, report that table — not raw per-frame lines. Raw
  lines are intermediate data, not the finding.
- Same removal rule applies once investigation concludes (see "Debug log
  conventions" in SKILL.md).
