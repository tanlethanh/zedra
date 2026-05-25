# Markdown Mermaid preview test

Fixture for fenced `mermaid` blocks in the workspace editor and terminal sheet preview. Each diagram uses **mock Zedra product content** (realistic labels, not empty boxes).

**Copy to a connected host:**

```bash
scp examples/mermaid.md your-host:/tmp/zedra-mermaid-test.md
# In the terminal, tap: /tmp/zedra-mermaid-test.md:1
```

Scroll the full file. Expect rendered diagrams (not monospace fences). Use **Show source** under a card to verify selection maps to the fence.

**Note:** Do not put Markdown code fences (three backticks) inside a `mermaid` fence.

---

## Flowchart — open README.md from terminal

Scenario: user taps an OSC-8 link to vendor/zed/README.md while zedra-host is connected.

```mermaid
flowchart TD
  tap["Tap /vendor/zed/README.md:1 in agent terminal"]
  sheet["Native sheet opens - TerminalPreviewView"]
  read["SessionHandle.fs_read path"]
  parse["background_spawn parse_markdown_source"]
  classify{"Fence language mermaid?"}
  mermaid["schedule_mermaid_renders - Pending card"]
  code["Code block - mono scroll"]
  render["mermaid-rs-renderer - SVG in ZedraAssets"]
  img["GPUI img - scaled diagram card"]
  tap --> sheet --> read --> parse --> classify
  classify -->|mermaid fence| mermaid --> render --> img
  classify -->|rust or bash| code
```

## Flowchart — session data path (LR)

Scenario: remote markdown bytes become pixels on the phone.

```mermaid
flowchart LR
  subgraph host["Mac - zedra-host"]
    disk["Repo file AGENTS.md"]
    rpc["RPC fs/read"]
  end
  subgraph phone["iOS - Zedra"]
    handle["SessionHandle"]
    state["WorkspaceState"]
    view["MarkdownView list rows"]
    cache["mermaid asset cache"]
  end
  disk --> rpc --> handle --> state --> view
  view --> cache
```

## Sequence — preview sheet load

```mermaid
sequenceDiagram
  actor Dev as Developer
  participant Term as Workspace terminal
  participant Sheet as Terminal preview sheet
  participant Sess as SessionHandle
  participant Host as zedra-host
  participant Mmd as mermaid-rs-renderer

  Dev->>Term: claude prints docs/MANUAL_TEST.md:42
  Term->>Sheet: open_file path + epoch
  Sheet->>Sess: fs_read /docs/MANUAL_TEST.md
  Sess->>Host: RPC ReadFile
  Host-->>Sess: UTF-8 markdown body
  Sess-->>Sheet: content + FileReady
  Sheet->>Sheet: parse_markdown_source (background)
  Note over Sheet: Block Mermaid at index 4
  Sheet->>Mmd: render flowchart TD
  Mmd-->>Sheet: SVG + viewBox
  Sheet-->>Dev: diagram card + Show source link
```

## Class — markdown preview model

```mermaid
classDiagram
  class MarkdownView {
    -document: MarkdownDocument
    -mermaid_states: HashMap
    -mermaid_generation: u64
    +set_parsed_source()
    +line_range_for_selection()
    +render()
  }
  class MarkdownDocument {
    +blocks: Block[]
    +selection_map
  }
  class Block {
    <<enumeration>>
    Paragraph
    CodeBlock
    Mermaid
    Table
  }
  class MermaidDiagram {
    +asset_path: String
    +intrinsic_width: f32
    +intrinsic_height: f32
  }
  MarkdownView --> MarkdownDocument
  MarkdownDocument *-- Block
  MarkdownView o-- MermaidDiagram : Ready state
```

## State — Mermaid block lifecycle

```mermaid
stateDiagram-v2
  [*] --> Idle : new MarkdownView
  Idle --> Pending : replace_document sees mermaid fence
  Pending --> Ready : SVG stored
  Pending --> Failed : parse or layout error
  Ready --> Pending : set_source new text
  Failed --> Pending : user fixes syntax
  Ready --> ShowingSource : tap Show source
  ShowingSource --> Ready : tap Hide source
  note right of Pending
    Placeholder card
    Rendering diagram
  end note
  note right of Failed
    Monospace fence
    Diagram could not be rendered
  end note
```

## ER — workspace on device

```mermaid
erDiagram
  SAVED_WORKSPACE ||--o{ SESSION : reconnects
  SESSION ||--|{ TERMINAL : multiplex
  SESSION ||--o{ AGENT_RUN : resumes
  TERMINAL ||--o{ PREVIEW_OPEN : osc_link
  PREVIEW_OPEN }o--|| REMOTE_FILE : fs_read

  SAVED_WORKSPACE {
    string display_name
    string node_id
    datetime last_connected
  }
  SESSION {
    string strip_path
    enum connect_phase
  }
  TERMINAL {
    string title
    string agent_icon
  }
  AGENT_RUN {
    string agent_kind
    string session_id
  }
  REMOTE_FILE {
    string path
    int size_bytes
  }
```

## Pie — time in workspace (mock telemetry)

```mermaid
pie title Where users spend time (beta cohort mock)
  "Terminal agent sessions" : 52
  "Markdown README preview" : 18
  "Code editor patch files" : 15
  "Git diff review" : 9
  "Agent manage resume" : 6
```

## Gantt — Mermaid preview rollout (fiction)

```mermaid
gantt
  title Zedra markdown preview mock schedule
  dateFormat  YYYY-MM-DD
  axisFormat %b %d

  section Foundation
  pulldown-cmark GFM tables     :done, gfm, 2026-04-01, 14d
  Virtualized MarkdownView      :done, list, 2026-04-10, 10d

  section Mermaid
  mermaid-rs-renderer spike     :done, spike, 2026-05-10, 3d
  Parse mermaid code fences     :done, parse, 2026-05-15, 4d
  Async render and img          :active, ui, 2026-05-20, 7d
  Manual test 15c on device     :crit, qa, after ui, 5d

  section Follow-up
  Viewport-aware diagram width  :active, width, after qa, 5d
```

## Git graph — feature branch (mock)

```mermaid
gitGraph
  commit id: "be648bf" tag: "v0.2.5" type: HIGHLIGHT
  branch feat-markdown-mermaid
  checkout feat-markdown-mermaid
  commit id: "feat-zedra-mermaid-parse-render"
  commit id: "examples-markdown-mermaid-fixtures"
  checkout main
  merge feat-markdown-mermaid
```

## Mindmap — Zedra mobile surface area

```mermaid
mindmap
  ((Zedra iOS))
    Home
      Scan QR pairing
      Saved workspaces
      Install guides
    Workspace
      Terminal grid
      File explorer
      Docs tree markdown only
      Git diff sidebar
    Preview
      CodeEditorView
      MarkdownView
        Tables task lists
        Mermaid diagrams
        OSC-8 links
      Sheet detent scroll
    Platform
      UIKit alerts sheets
      Metal GPUI
      Haptics light impact
```

## Journey — first Mermaid README (mock persona)

```mermaid
journey
  title Alex ships a diagram in internal README
  section Commute
    Open Zedra on LTE to home Mac: 4: Alex
    Terminal shows agent done and file link: 5: Alex
  section Preview
    Tap link sheet loads README: 4: Alex
    Scroll past table see flowchart: 5: Alex
    Pinch mentally checks legibility: 3: Alex
  section Share
    Long-press diagram source: 4: Alex
    Add selection to chat with agent: 5: Alex
```

## Timeline — markdown capabilities

```mermaid
timeline
  title Zedra markdown preview history (mock)
  section 2025
    Q3 : Terminal OSC-8 file links
    Q4 : Custom sheet code preview
  section 2026
    Q1 : Workspace markdown editor mode
    Q2 : GFM tables and task lists
         : Mermaid render via mermaid-rs-renderer
```

## Quadrant — diagram renderers considered

```mermaid
quadrantChart
  title Mobile markdown diagram backends
  x-axis Low implementation cost --> High cost
  y-axis Low visual fidelity --> High fidelity
  quadrant-1 Quick win
  quadrant-2 Heavyweight
  quadrant-3 Avoid
  quadrant-4 Ideal long-term
  "mermaid-rs-renderer embedded SVG": [0.35, 0.72]
  "WKWebView mermaid.js CDN": [0.55, 0.88]
  "Host RPC pre-render PNG": [0.75, 0.8]
  "Monospace fence only": [0.15, 0.2]
```

## XY chart — preview scroll FPS vs doc size (mock)

```mermaid
xychart-beta
  title "Markdown list scroll FPS by doc size (mock lab)"
  x-axis ["8 blocks", "40 blocks", "120 blocks with mermaid"]
  y-axis "median FPS" 50 --> 120
  line [112, 96, 78]
  bar [110, 94, 74]
```

## Sequence — stale render guard

```mermaid
sequenceDiagram
  participant View as MarkdownView
  participant Gen as mermaid_generation
  participant Task as background task

  View->>Gen: increment on replace_document
  View->>Task: spawn render block 2
  Note over View: User opens different file
  View->>Gen: increment again
  Task-->>View: render complete (old gen)
  View->>View: drop result gen mismatch
  View->>Task: spawn render block 0
  Task-->>View: render complete (current gen)
  View->>View: insert Ready notify
```

## Intentional failure (fallback)

Invalid syntax — expect monospace fence plus error line.

```mermaid
diagram-type-that-does-not-exist
  Zedra --> should not render
```

## Control — Rust fence (not Mermaid)

```rust
// This block must stay a horizontal-scroll code card.
pub fn is_markdown_path(path: &str) -> bool {
    path.ends_with(".md") || path.eq_ignore_ascii_case("readme")
}
```
