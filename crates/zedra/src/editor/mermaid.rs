use mermaid_rs_renderer::{LayoutConfig, RenderOptions, Theme, render_with_options};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use crate::theme::{ThemePalette, ThemePreference};

const MERMAID_ASSET_PREFIX: &str = "mermaid/";

/// Maps Zedra UI palette tokens into `mermaid-rs-renderer` [`Theme`] fields.
struct MermaidPalette {
    canvas: u32,
    node_fill: u32,
    node_fill_alt: u32,
    node_border: u32,
    ink: u32,
    ink_muted: u32,
    edge_stroke: u32,
    note_fill: u32,
}

impl MermaidPalette {
    fn for_preference(preference: ThemePreference) -> Self {
        let ui = match preference {
            ThemePreference::Dark => ThemePalette::dark(),
            ThemePreference::Light => ThemePalette::light(),
        };
        Self::from_ui(&ui, preference)
    }

    fn from_ui(ui: &ThemePalette, preference: ThemePreference) -> Self {
        let note_fill = match preference {
            ThemePreference::Dark => 0x2a2618,
            ThemePreference::Light => 0xfff7ed,
        };
        let node_fill_alt = match preference {
            ThemePreference::Dark => 0x1f1f1f,
            ThemePreference::Light => ui.border_subtle,
        };
        // Dark: bg_card=#131313, border_subtle=#1a1a1a — only 7-point delta,
        // nodes invisible against canvas. Use a brighter fill so shapes pop.
        let node_fill = match preference {
            ThemePreference::Dark => 0x2c2c2c,
            ThemePreference::Light => ui.border_subtle,
        };
        // Dark: bright accent_blue (#61afef) is visually heavy on dark canvas.
        // Use a dim neutral so arrows recede behind node labels.
        let edge_stroke = match preference {
            ThemePreference::Dark => 0x888888,
            ThemePreference::Light => ui.accent_blue,
        };
        Self {
            canvas: ui.bg_card,
            node_fill,
            node_fill_alt,
            node_border: ui.border_default,
            ink: ui.text_primary,
            ink_muted: ui.text_secondary,
            edge_stroke,
            note_fill,
        }
    }
}

/// In-memory SVG assets for dynamically rendered Mermaid diagrams (`img()` embedded paths).
static MERMAID_SVG_CACHE: OnceLock<Mutex<HashMap<String, Arc<[u8]>>>> = OnceLock::new();

pub fn mermaid_asset_path(block_ix: usize, source: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{MERMAID_ASSET_PREFIX}{block_ix}-{:x}.svg", hasher.finish())
}

pub fn store_mermaid_svg(path: String, bytes: Arc<[u8]>) {
    MERMAID_SVG_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("mermaid svg cache lock")
        .insert(path, bytes);
}

pub fn load_mermaid_svg(path: &str) -> Option<Arc<[u8]>> {
    MERMAID_SVG_CACHE
        .get()
        .and_then(|cache| cache.lock().ok().and_then(|g| g.get(path).cloned()))
}

pub fn clear_mermaid_svg_cache() {
    if let Some(cache) = MERMAID_SVG_CACHE.get() {
        cache.lock().expect("mermaid svg cache lock").clear();
    }
}

pub fn is_mermaid_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("mermaid")
}

#[derive(Clone, Debug)]
pub struct MermaidDiagram {
    pub asset_path: String,
    pub intrinsic_width: f32,
    pub intrinsic_height: f32,
}

pub fn render_mermaid_diagram(
    block_ix: usize,
    source: &str,
    preference: ThemePreference,
) -> Option<MermaidDiagram> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    let kind = detect_diagram_kind(source);

    let svg = match kind {
        DiagramKind::Timeline => render_zedra_timeline_svg(source, preference)?,
        _ => {
            let mut svg =
                render_with_options(source, mermaid_render_options_for(kind, source, preference))
                    .ok()?;
            if kind == DiagramKind::Quadrant {
                svg = post_process_quadrant_svg(&svg, preference);
            }
            svg
        }
    };
    let (intrinsic_width, intrinsic_height) = svg_intrinsic_size(&svg)?;
    let asset_path = mermaid_asset_path(block_ix, source);
    store_mermaid_svg(asset_path.clone(), Arc::from(svg.into_bytes()));

    Some(MermaidDiagram {
        asset_path,
        intrinsic_width,
        intrinsic_height,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiagramKind {
    Timeline,
    Quadrant,
    State,
    Journey,
    Other,
}

fn detect_diagram_kind(source: &str) -> DiagramKind {
    let first = match source.lines().find(|l| !l.trim().is_empty()) {
        Some(l) => l.trim_start(),
        None => return DiagramKind::Other,
    };
    if first.starts_with("timeline") {
        DiagramKind::Timeline
    } else if first.starts_with("quadrantChart") {
        DiagramKind::Quadrant
    } else if first.starts_with("stateDiagram") {
        DiagramKind::State
    } else if first.starts_with("journey") {
        DiagramKind::Journey
    } else {
        DiagramKind::Other
    }
}

pub(crate) fn mermaid_render_options_for(
    kind: DiagramKind,
    source: &str,
    preference: ThemePreference,
) -> RenderOptions {
    let mut options = mermaid_render_options(preference);
    // State diagrams skip the library's flowchart_label_spacing_floor heuristic
    // (it only runs for DiagramKind::Flowchart). Even when we boost spacing,
    // adaptive_spacing_for_nodes then reduces it back to ~avg_node_size*0.5.
    // Disable auto_spacing so our explicit values are preserved, then set spacing
    // proportional to the longest transition label.
    if kind == DiagramKind::State {
        let max_label_chars = longest_edge_label_chars(source);
        let boost = ((max_label_chars as f32 - 10.0) / 24.0).clamp(0.0, 1.0);
        let extra = boost * 60.0;
        options.layout.flowchart.auto_spacing.enabled = false;
        options.layout.rank_spacing = 50.0 + extra;
        // Extra node_spacing gives bidirectional edge pairs (e.g. Ready↔ShowingSource)
        // more horizontal separation so their labels don't stack.
        options.layout.node_spacing = 50.0 + extra * 0.8;
    }
    if kind == DiagramKind::Journey {
        configure_journey_layout(&mut options);
    }
    options
}

pub(crate) fn mermaid_render_options(preference: ThemePreference) -> RenderOptions {
    let ui = match preference {
        ThemePreference::Dark => ThemePalette::dark(),
        ThemePreference::Light => ThemePalette::light(),
    };
    let mut options = RenderOptions::default();
    options.theme = mermaid_theme_for_preference(&ui, preference);
    configure_flowchart_layout(&mut options.layout);
    // Gantt row_height is fixed (one line); raising this cap prevents task
    // labels from wrapping and bleeding into adjacent rows.
    options.layout.max_label_width_chars = 50;
    configure_pie_layout(&mut options.layout);
    configure_gitgraph_layout(&mut options.layout);
    configure_mindmap_layout(&mut options.layout, &ui, preference);
    options
}

// Post-processing for quadrant charts: mermaid-rs-renderer hardcodes light
// purple backgrounds and borders that ignore the theme. We swap them in a
// single pass after rendering.
//
// This is context-blind string replacement. If a user label happens to contain
// one of the old hex strings, it would also be swapped. This is acceptable
// because the old colors are mermaid-internal pastel shades (e.g. #ECECFF)
// that are vanishingly unlikely to appear in user text.
const QUADRANT_DARK_REPLACEMENTS: [(&str, &str); 12] = [
    ("#ECECFF", "#1a1a1a"),
    ("#f1f1ff", "#1d1d1d"),
    ("#f6f6ff", "#202020"),
    ("#fbfbff", "#232323"),
    ("#c7c7f1", "#505050"),
    ("#131300", "#ffffff"),
    ("#6366f1", "#61afef"),
    ("#f59e0b", "#98c379"),
    ("#10b981", "#e5c07b"),
    ("#ef4444", "#e06c75"),
    ("#8b5cf6", "#cacaca"),
    ("#06b6d4", "#61afef"),
];

const QUADRANT_LIGHT_REPLACEMENTS: [(&str, &str); 12] = [
    ("#ECECFF", "#f8f9fa"),
    ("#f1f1ff", "#f5f6f7"),
    ("#f6f6ff", "#f2f3f4"),
    ("#fbfbff", "#eff0f1"),
    ("#c7c7f1", "#d8d8d8"),
    ("#131300", "#1a1a1a"),
    ("#6366f1", "#0969da"),
    ("#f59e0b", "#1a7f37"),
    ("#10b981", "#9a6700"),
    ("#ef4444", "#cf222e"),
    ("#8b5cf6", "#8a8a8a"),
    ("#06b6d4", "#0969da"),
];

fn post_process_quadrant_svg(svg: &str, preference: ThemePreference) -> String {
    let table: &[(&str, &str)] = match preference {
        ThemePreference::Dark => &QUADRANT_DARK_REPLACEMENTS,
        ThemePreference::Light => &QUADRANT_LIGHT_REPLACEMENTS,
    };
    let mut result = svg.to_string();
    for (old, new) in table {
        result = result.replace(old, new);
    }
    result
}

/// Approximate longest edge label by scanning lines with `-->` or `:` transitions.
fn longest_edge_label_chars(source: &str) -> usize {
    source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            // Matches `A --> B : label` and `A --> B: label`
            let colon_pos = trimmed.find(':')?;
            let before = &trimmed[..colon_pos];
            if before.contains("-->") || before.contains("->") {
                Some(trimmed[colon_pos + 1..].trim().chars().count())
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
}

fn configure_flowchart_layout(layout: &mut LayoutConfig) {
    // High turn_penalty strongly discourages diagonal routing so branch nodes
    // stay in the same column and arrows remain straight.
    layout.flowchart.routing.turn_penalty = 5.0;
    layout.flowchart.routing.grid_cell = 12.0;
    // More ordering and relaxation passes improve Sugiyama layer alignment.
    layout.flowchart.order_passes = 8;
    layout.flowchart.objective.edge_relax_passes = 6;
    // Narrow aspect ratio keeps TD flowcharts portrait — discourages the
    // layout from spreading branch columns horizontally.
    layout.flowchart.objective.max_aspect_ratio = 2.5;
}

fn configure_pie_layout(layout: &mut LayoutConfig) {
    // Default height=360 → center_x=180. A long title centered at 180 with
    // text-anchor="middle" clips on the left (title half-width > center_x).
    // Increasing height widens the pie canvas so center_x has more headroom.
    layout.pie.height = 500.0;
    // legend_rect_size matched to legend text size so swatches fill the same
    // vertical span as the text, reducing the visual misalignment.
    layout.pie.legend_rect_size = 15.0;
    layout.pie.legend_spacing = 5.0;
}

fn configure_gitgraph_layout(layout: &mut LayoutConfig) {
    let git = &mut layout.gitgraph;
    git.rotate_commit_label = false;
    git.commit_step = 52.0;
    git.branch_spacing = 48.0;
    git.branch_label_font_size = 12.0;
    git.commit_label_font_size = 11.0;
    git.commit_label_offset_y = 18.0;
    git.commit_label_bg_offset_y = 6.5;
    git.commit_label_padding = 4.0;
    git.commit_label_bg_opacity = 0.92;
    git.tag_label_font_size = 11.0;
    git.arrow_stroke_width = 4.0;
    git.branch_stroke_width = 0.6;
    git.branch_dasharray = "3 4".to_string();
    git.commit_radius = 6.0;
    git.merge_radius_outer = 6.5;
    git.merge_radius_inner = 4.0;
    git.highlight_outer_size = 13.0;
    git.highlight_inner_size = 7.0;
}

fn configure_mindmap_layout(
    layout: &mut LayoutConfig,
    ui: &ThemePalette,
    preference: ThemePreference,
) {
    let mindmap = &mut layout.mindmap;
    mindmap.padding = 20.0;
    mindmap.max_node_width = 170.0;
    mindmap.node_spacing = 36.0;
    mindmap.rank_spacing = 34.0;
    mindmap.node_spacing_multiplier = 0.85;
    mindmap.rank_spacing_multiplier = 0.85;
    mindmap.rounded_padding = 10.0;
    mindmap.rect_padding = 8.0;
    mindmap.circle_padding = 14.0;
    mindmap.default_corner_radius = 5.0;
    mindmap.edge_depth_base_width = 7.0;
    mindmap.edge_depth_step = -1.1;
    mindmap.divider_line_width = 1.2;
    mindmap.section_colors = mindmap_section_colors(ui, preference).to_vec();
    mindmap.section_label_colors = mindmap_section_label_colors(ui).to_vec();
    mindmap.section_line_colors = mindmap_section_line_colors(ui, preference).to_vec();
    mindmap.root_fill = Some(match preference {
        ThemePreference::Dark => "#34465c".to_string(),
        ThemePreference::Light => hex_color(ui.accent_blue),
    });
    mindmap.root_text = Some(match preference {
        ThemePreference::Dark => hex_color(ui.text_primary),
        ThemePreference::Light => hex_color(ui.bg_card),
    });
}

fn configure_journey_layout(options: &mut RenderOptions) {
    // Journey rows use one fixed card width based on the widest task. Mobile
    // fixtures with sentence-length tasks otherwise create very wide SVGs that
    // crop inside the preview card, leaving most of the card blank.
    options.layout.max_label_width_chars = 12;
    options.layout.label_line_height = 1.25;
    options.theme.font_size = 13.0;
    options.layout.preferred_aspect_ratio = Some(2.2);
}

pub(crate) fn mermaid_theme_for_preference(
    ui: &ThemePalette,
    preference: ThemePreference,
) -> Theme {
    let palette = MermaidPalette::from_ui(ui, preference);
    let canvas = hex_color(palette.canvas);
    let node = hex_color(palette.node_fill);
    let node_alt = hex_color(palette.node_fill_alt);
    let node_border = hex_color(palette.node_border);
    let ink = hex_color(palette.ink);
    let ink_muted = hex_color(palette.ink_muted);
    let edge = hex_color(palette.edge_stroke);
    let note = hex_color(palette.note_fill);

    let mut theme = Theme::modern();
    // GPUI's SVG renderer (usvg) only has Lora + JetBrains Mono bundled; IBM
    // Plex Sans is absent, causing usvg to pick a different sans fallback than
    // text_metrics does — sizing mismatch. "sans-serif" resolves to the same
    // family in both paths.
    theme.font_family = "sans-serif".to_string();
    // Pie title is centered at x=height/2 (180 default). At 25px a long title
    // overflows left. Reduce size so the text fits within the wider canvas
    // set by configure_pie_layout (height=500 → center_x=250).
    theme.pie_title_text_size = 19.0;
    theme.pie_legend_text_size = 15.0;
    theme.background = canvas.clone();

    theme.primary_color = node.clone();
    theme.secondary_color = node_alt.clone();
    theme.tertiary_color = node.clone();
    theme.primary_text_color = ink.clone();
    theme.text_color = ink.clone();
    theme.primary_border_color = node_border.clone();
    theme.line_color = edge.clone();
    theme.edge_label_background = node_alt.clone();
    theme.cluster_background = node_alt.clone();
    theme.cluster_border = node_border.clone();

    theme.sequence_actor_fill = node.clone();
    theme.sequence_actor_border = node_border.clone();
    theme.sequence_actor_line = ink_muted.clone();
    theme.sequence_note_fill = note;
    theme.sequence_note_border = node_border.clone();
    theme.sequence_activation_fill = node_alt.clone();
    theme.sequence_activation_border = node_border.clone();

    theme.pie_colors = pie_colors(ui, preference);
    theme.pie_title_text_color = ink.clone();
    theme.pie_section_text_color = ink.clone();
    theme.pie_legend_text_color = ink.clone();
    theme.pie_stroke_color = node_border.clone();
    theme.pie_outer_stroke_color = node_border.clone();

    theme.git_colors = git_colors(ui, preference);
    theme.git_inv_colors = git_inverse_colors(ui, preference);
    theme.git_branch_label_colors = git_branch_label_colors(ui, preference);
    theme.git_commit_label_color = ink.clone();
    theme.git_commit_label_background = node_alt.clone();
    theme.git_tag_label_color = ink.clone();
    theme.git_tag_label_background = node_alt.clone();
    theme.git_tag_label_border = node_border.clone();

    theme
}

fn hex_color(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xffffff)
}

fn pie_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 12] {
    match preference {
        ThemePreference::Dark => [
            hex_color(ui.accent_blue),
            hex_color(ui.accent_green),
            hex_color(ui.accent_yellow),
            hex_color(ui.accent_red),
            "#4a6a8a".to_string(),
            "#5a7a5a".to_string(),
            "#8a7a4a".to_string(),
            "#8a5a5a".to_string(),
            "#3d5a80".to_string(),
            "#4a6b4a".to_string(),
            "#6b5a3d".to_string(),
            "#6b4a4a".to_string(),
        ],
        ThemePreference::Light => Theme::modern().pie_colors,
    }
}

fn git_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 8] {
    match preference {
        ThemePreference::Dark => [
            "#4f84b8".to_string(),
            "#8a8a52".to_string(),
            hex_color(ui.accent_green),
            "#a06c72".to_string(),
            "#6f8c8c".to_string(),
            "#7d7da8".to_string(),
            "#8a6f8c".to_string(),
            hex_color(ui.accent_dim),
        ],
        ThemePreference::Light => [
            hex_color(ui.accent_blue),
            hex_color(ui.accent_yellow),
            hex_color(ui.accent_green),
            hex_color(ui.accent_red),
            "#4f7f7f".to_string(),
            "#6666a3".to_string(),
            "#8a5c8a".to_string(),
            hex_color(ui.accent_dim),
        ],
    }
}

fn git_inverse_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 8] {
    let inverse = match preference {
        ThemePreference::Dark => ui.bg_card,
        ThemePreference::Light => ui.bg_primary,
    };
    std::array::from_fn(|_| hex_color(inverse))
}

fn git_branch_label_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 8] {
    match preference {
        ThemePreference::Dark => [
            hex_color(ui.text_primary),
            hex_color(ui.bg_card),
            hex_color(ui.bg_card),
            hex_color(ui.text_primary),
            hex_color(ui.bg_card),
            hex_color(ui.text_primary),
            hex_color(ui.text_primary),
            hex_color(ui.text_primary),
        ],
        ThemePreference::Light => std::array::from_fn(|_| hex_color(ui.bg_card)),
    }
}

fn mindmap_section_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 12] {
    match preference {
        ThemePreference::Dark => [
            "#3a4a34".to_string(),
            "#3e364d".to_string(),
            "#34465c".to_string(),
            "#4a3838".to_string(),
            "#354846".to_string(),
            "#4b4436".to_string(),
            "#3f3f52".to_string(),
            "#43384a".to_string(),
            "#344a3c".to_string(),
            "#4a3d35".to_string(),
            "#37454e".to_string(),
            "#4a4040".to_string(),
        ],
        ThemePreference::Light => [
            "#dce8d2".to_string(),
            "#e8ddf0".to_string(),
            "#d7e5f5".to_string(),
            "#f0dddd".to_string(),
            "#d9e9e5".to_string(),
            "#ebe4d6".to_string(),
            "#dedff0".to_string(),
            "#eaddec".to_string(),
            "#d8e9df".to_string(),
            "#eee2d8".to_string(),
            "#dbe6ec".to_string(),
            hex_color(ui.border_subtle),
        ],
    }
}

fn mindmap_section_label_colors(ui: &ThemePalette) -> [String; 12] {
    std::array::from_fn(|_| hex_color(ui.text_primary))
}

fn mindmap_section_line_colors(ui: &ThemePalette, preference: ThemePreference) -> [String; 12] {
    match preference {
        ThemePreference::Dark => [
            "#6f8f62".to_string(),
            "#8a6aa1".to_string(),
            "#5f84ad".to_string(),
            "#9b6f6f".to_string(),
            "#69918b".to_string(),
            "#9a855d".to_string(),
            "#7778a4".to_string(),
            "#8e6a94".to_string(),
            "#6e967a".to_string(),
            "#9a7b61".to_string(),
            "#6f8fa4".to_string(),
            "#8a7777".to_string(),
        ],
        ThemePreference::Light => mindmap_section_colors(ui, preference),
    }
}

struct TimelineEntry {
    time: String,
    events: Vec<String>,
}

struct TimelineData {
    title: Option<String>,
    entries: Vec<TimelineEntry>,
}

fn render_zedra_timeline_svg(source: &str, preference: ThemePreference) -> Option<String> {
    let data = parse_timeline_source(source);
    if data.entries.is_empty() {
        return None;
    }

    let palette = MermaidPalette::for_preference(preference);
    let bg = hex_color(palette.canvas);
    let card = hex_color(palette.node_fill);
    let border = hex_color(palette.node_border);
    let ink = hex_color(palette.ink);
    let muted = hex_color(palette.ink_muted);
    let line = hex_color(palette.edge_stroke);
    let colors = timeline_colors_for_preference(preference);

    let title_size = 14.0;
    let label_size = 12.0;
    let time_size = 12.0;
    let margin_x = 32.0;
    let title_height = if data.title.is_some() { 34.0 } else { 8.0 };
    let line_y = title_height + 58.0;
    let card_y = line_y + 36.0;
    let card_width = 150.0;
    let card_height = 86.0;
    let gap = 28.0;
    let width = margin_x * 2.0
        + data.entries.len() as f32 * card_width
        + data.entries.len().saturating_sub(1) as f32 * gap;
    let height = card_y + card_height + 28.0;
    let line_start = margin_x + card_width / 2.0;
    let line_end = width - margin_x - card_width / 2.0;

    // Rough capacity: ~400 bytes fixed overhead + ~500 bytes per entry.
    let estimated = 400 + data.entries.len() * 500;
    let mut svg = String::with_capacity(estimated);
    use std::fmt::Write;
    let _ = write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.2}" height="{height:.2}" viewBox="0 0 {width:.2} {height:.2}" style="max-width: {width:.2}px;" font-family="sans-serif">"#
    );
    let _ = write!(
        svg,
        r#"<rect x="0" y="0" width="{width:.2}" height="{height:.2}" fill="{}"/>"#,
        escape_xml(&bg)
    );

    if let Some(title) = data.title.as_deref() {
        write_svg_text(
            &mut svg,
            width / 2.0,
            24.0,
            title,
            title_size,
            &ink,
            "middle",
            Some("600"),
        );
    }

    let _ = write!(
        svg,
        r#"<line x1="{line_start:.2}" y1="{line_y:.2}" x2="{line_end:.2}" y2="{line_y:.2}" stroke="{}" stroke-width="2" stroke-linecap="round"/>"#,
        escape_xml(&line)
    );

    for (ix, entry) in data.entries.iter().enumerate() {
        let x = margin_x + ix as f32 * (card_width + gap);
        let center_x = x + card_width / 2.0;
        let color = colors[ix % colors.len()];
        let _ = write!(
            svg,
            r#"<line x1="{center_x:.2}" y1="{:.2}" x2="{center_x:.2}" y2="{:.2}" stroke="{}" stroke-width="1" stroke-dasharray="3 4"/>"#,
            line_y + 9.0,
            card_y,
            escape_xml(&border)
        );
        let _ = write!(
            svg,
            r#"<circle cx="{center_x:.2}" cy="{line_y:.2}" r="8" fill="{}" stroke="{}" stroke-width="2"/>"#,
            escape_xml(&card),
            escape_xml(color)
        );
        let _ = write!(
            svg,
            r#"<rect x="{x:.2}" y="{card_y:.2}" width="{card_width:.2}" height="{card_height:.2}" rx="6" ry="6" fill="{}" stroke="{}" stroke-width="1"/>"#,
            escape_xml(&card),
            escape_xml(&border)
        );
        let _ = write!(
            svg,
            r#"<rect x="{x:.2}" y="{card_y:.2}" width="{card_width:.2}" height="26" rx="6" ry="6" fill="{}" fill-opacity="0.35"/>"#,
            escape_xml(color)
        );
        write_svg_text(
            &mut svg,
            center_x,
            card_y + 18.0,
            &entry.time,
            time_size,
            &ink,
            "middle",
            Some("600"),
        );

        let mut y = card_y + 43.0;
        for event in &entry.events {
            for line in wrap_text_words(event, 18).into_iter().take(3) {
                write_svg_text(
                    &mut svg, center_x, y, &line, label_size, &muted, "middle", None,
                );
                y += label_size * 1.2;
            }
        }
    }

    svg.push_str("</svg>");
    Some(svg)
}

fn parse_timeline_source(source: &str) -> TimelineData {
    let mut title = None;
    let mut entries = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed == "timeline" {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("title ") {
            title = Some(value.trim_matches('"').to_string());
            continue;
        }
        if trimmed.starts_with("section ") {
            continue;
        }
        let mut parts = trimmed
            .split(':')
            .map(str::trim)
            .filter(|part| !part.is_empty());
        let Some(time) = parts.next() else {
            continue;
        };
        let events: Vec<String> = parts
            .map(|part| part.trim_matches('"').to_string())
            .collect();
        if !events.is_empty() {
            entries.push(TimelineEntry {
                time: time.to_string(),
                events,
            });
        }
    }
    TimelineData { title, entries }
}

fn wrap_text_words(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        // A single word may exceed max_chars; split it so the fixed-width
        // card (150 px at 12 px font) does not overflow.
        if word_len > max_chars {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
                current_len = 0;
            }
            for c in word.chars() {
                current.push(c);
                current_len += 1;
                if current_len == max_chars {
                    lines.push(current);
                    current = String::new();
                    current_len = 0;
                }
            }
            continue;
        }
        let next_len = if current.is_empty() {
            word_len
        } else {
            current_len + 1 + word_len
        };
        if next_len > max_chars && !current.is_empty() {
            lines.push(current);
            current = String::new();
            current_len = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn write_svg_text(
    svg: &mut String,
    x: f32,
    y: f32,
    text: &str,
    size: f32,
    color: &str,
    anchor: &str,
    weight: Option<&str>,
) {
    use std::fmt::Write;
    let weight_attr = weight
        .map(|value| format!(r#" font-weight="{}""#, escape_xml(value)))
        .unwrap_or_default();
    let _ = write!(
        svg,
        r#"<text x="{x:.2}" y="{y:.2}" text-anchor="{}" font-size="{size:.1}" fill="{}"{}>{}</text>"#,
        escape_xml(anchor),
        escape_xml(color),
        weight_attr,
        escape_xml(text)
    );
}

fn timeline_colors_for_preference(preference: ThemePreference) -> [&'static str; 6] {
    match preference {
        ThemePreference::Dark => [
            "#4f84b8", "#8a8a52", "#6f8f62", "#8a6aa1", "#9b6f6f", "#69918b",
        ],
        ThemePreference::Light => [
            "#d7e5f5", "#ebe4d6", "#dce8d2", "#e8ddf0", "#f0dddd", "#d9e9e5",
        ],
    }
}

fn escape_xml(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            _ => result.push(c),
        }
    }
    result
}

/// Best-effort intrinsic size from SVG `viewBox` or `width`/`height` attributes.
fn svg_intrinsic_size(svg: &str) -> Option<(f32, f32)> {
    if let Some((w, h)) = parse_view_box(svg) {
        if w > 0.0 && h > 0.0 {
            return Some((w, h));
        }
    }

    let width = parse_svg_length_attr(svg, "width")?;
    let height = parse_svg_length_attr(svg, "height")?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn parse_view_box(svg: &str) -> Option<(f32, f32)> {
    let marker = "viewBox=\"";
    let start = svg.find(marker)? + marker.len();
    let end = svg[start..].find('"')? + start;
    let mut parts = svg[start..end].split_whitespace();
    let _min_x = parts.next()?;
    let _min_y = parts.next()?;
    let width: f32 = parts.next()?.parse().ok()?;
    let height: f32 = parts.next()?.parse().ok()?;
    Some((width, height))
}

fn parse_svg_length_attr(svg: &str, attr: &str) -> Option<f32> {
    let marker = format!("{attr}=\"");
    let start = svg.find(&marker)? + marker.len();
    let end = svg[start..].find('"')? + start;
    let raw = &svg[start..end];
    raw.trim_end_matches("px").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_svg_view_box_dimensions() {
        let svg = r#"<svg viewBox="0 0 320 180" xmlns="http://www.w3.org/2000/svg"></svg>"#;
        assert_eq!(svg_intrinsic_size(svg), Some((320.0, 180.0)));
    }

    #[test]
    fn dark_theme_uses_dark_nodes_and_light_canvas_text() {
        let theme = mermaid_theme_for_preference(&ThemePalette::dark(), ThemePreference::Dark);
        assert_eq!(theme.background, "#131313");
        assert_eq!(theme.primary_color, "#2c2c2c");
        assert_eq!(theme.primary_text_color, "#ffffff");
        assert_eq!(theme.pie_legend_text_color, "#ffffff");
        assert_eq!(theme.text_color, "#ffffff");
        assert_eq!(theme.line_color, "#888888");
    }

    #[test]
    fn dark_gitgraph_uses_zedra_palette_and_readable_labels() {
        let options = mermaid_render_options(ThemePreference::Dark);
        assert_eq!(options.theme.git_colors[0], "#4f84b8");
        assert_eq!(options.theme.git_colors[1], "#8a8a52");
        assert_eq!(options.theme.git_commit_label_color, "#ffffff");
        assert_eq!(options.theme.git_commit_label_background, "#1f1f1f");
        assert!(!options.layout.gitgraph.rotate_commit_label);
        assert_eq!(options.layout.gitgraph.commit_label_font_size, 11.0);

        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            7,
            r#"gitGraph
  commit id: "be648bf" tag: "v0.2.5" type: HIGHLIGHT
  branch feat-markdown-mermaid
  checkout feat-markdown-mermaid
  commit id: "feat-zedra-mermaid-parse-render"
  checkout main
  merge feat-markdown-mermaid"#,
            ThemePreference::Dark,
        )
        .expect("gitGraph should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            !svg.contains("rotate(-45"),
            "commit labels should not be rotated on mobile"
        );
        assert!(
            !svg.contains("hsl(240, 100%, 46.2745098039%)"),
            "should not use Mermaid default gitGraph lane colors"
        );
    }

    #[test]
    fn dark_mindmap_uses_compact_muted_theme() {
        let options = mermaid_render_options(ThemePreference::Dark);
        assert_eq!(options.layout.mindmap.section_colors[0], "#3a4a34");
        assert_eq!(options.layout.mindmap.root_fill.as_deref(), Some("#34465c"));
        assert_eq!(options.layout.mindmap.edge_depth_base_width, 7.0);
        assert_eq!(options.layout.mindmap.max_node_width, 170.0);

        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            8,
            r#"mindmap
  ((Zedra iOS))
    Workspace
      Terminal grid
      File explorer
      Docs tree markdown only
      Git diff sidebar
    Platform
      UIKit alerts sheets
      Metal GPUI
      Haptics light impact"#,
            ThemePreference::Dark,
        )
        .expect("mindmap should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            !svg.contains("hsl(60, 100%, 73.5294117647%)"),
            "should not use Mermaid default neon mindmap fills"
        );
        assert!(
            !svg.contains("stroke-width=\"11"),
            "mindmap branch strokes should be compact on mobile"
        );
    }

    #[test]
    fn journey_wraps_task_labels_for_mobile_preview() {
        let source = r#"journey
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
    Add selection to chat with agent: 5: Alex"#;
        let options =
            mermaid_render_options_for(DiagramKind::Journey, source, ThemePreference::Dark);
        assert_eq!(options.layout.max_label_width_chars, 12);
        assert_eq!(options.layout.label_line_height, 1.25);
        assert_eq!(options.layout.preferred_aspect_ratio, Some(2.2));

        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(9, source, ThemePreference::Dark)
            .expect("journey should render");
        assert!(
            diagram.intrinsic_width < 1500.0,
            "journey should wrap task cards instead of rendering an oversized row: {}",
            diagram.intrinsic_width
        );
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            svg.contains("Terminal") && svg.contains("agent") && svg.contains("done"),
            "wrapped labels should retain task text"
        );
    }

    #[test]
    fn timeline_wraps_labels_inside_event_cards() {
        let source = r#"timeline
  title Zedra markdown preview history (mock)
  section 2025
    Q3 : Terminal OSC-8 file links
    Q4 : Custom sheet code preview
  section 2026
    Q1 : Workspace markdown editor mode
    Q2 : GFM tables and task lists
         : Mermaid render via mermaid-rs-renderer"#;

        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(10, source, ThemePreference::Dark)
            .expect("timeline should render");
        assert!(
            diagram.intrinsic_width < 900.0,
            "timeline should stay card-sized: {}",
            diagram.intrinsic_width
        );
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(svg.contains("Terminal OSC-8"));
        assert!(svg.contains("file links"));
        assert!(
            !svg.contains("#ECECFF"),
            "timeline should not use Mermaid default pastel cards"
        );
    }

    #[test]
    fn diagrams_md_blocks_parse_and_render() {
        use mermaid_rs_renderer::parse_mermaid;
        use std::fs;

        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/mermaid.md");
        let md = fs::read_to_string(&path).expect("diagrams fixture");
        let mut current_section = String::from("preamble");
        let mut in_fence = false;
        let mut source = String::new();
        for line in md.lines() {
            if let Some(title) = line.strip_prefix("## ") {
                if in_fence {
                    assert_block(&current_section, &source);
                    in_fence = false;
                    source.clear();
                }
                current_section = title.to_string();
                continue;
            }
            if line.trim() == "```mermaid" {
                in_fence = true;
                source.clear();
                continue;
            }
            if in_fence && line.trim() == "```" {
                assert_block(&current_section, &source);
                in_fence = false;
                source.clear();
                continue;
            }
            if in_fence {
                if !source.is_empty() {
                    source.push('\n');
                }
                source.push_str(line);
            }
        }

        fn assert_block(section: &str, source: &str) {
            if section.contains("Intentional failure") {
                return;
            }
            parse_mermaid(source).unwrap_or_else(|err| {
                panic!("parse failed for [{section}]: {err}\n---\n{source}\n---")
            });
            clear_mermaid_svg_cache();
            render_mermaid_diagram(0, source, ThemePreference::Dark)
                .unwrap_or_else(|| panic!("render failed for [{section}]\n---\n{source}\n---"));
        }
    }

    #[test]
    fn flowchart_edges_use_dim_stroke_in_dark_mode() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            9,
            "flowchart TD\n  A[One] --> B[Two]\n  B --> C{Pick}\n  C -->|yes| D[Yes]\n  C -->|no| E[No]",
            ThemePreference::Dark,
        )
        .expect("flowchart should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            svg.contains("stroke=\"#888888\""),
            "expected dim neutral edge stroke in dark mode"
        );
    }

    #[test]
    fn dark_er_diagram_uses_light_ink_on_dark_entity_body() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            0,
            "erDiagram\n  A ||--o{ B : rel\n  A { string id }\n  B { string id }",
            ThemePreference::Dark,
        )
        .expect("er should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            svg.contains("fill=\"#ffffff\"") || svg.contains("fill=\"#FFFFFF\""),
            "expected light label ink"
        );
        assert!(
            !svg.contains("fill=\"#F8FAFC\"") && !svg.contains("fill=\"#f8fafc\""),
            "should not use modern() light actor fills"
        );
    }

    #[test]
    fn rendered_sequence_svg_includes_text_labels() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            1,
            "sequenceDiagram\n  participant Dev as Developer\n  participant Zedra\n  Dev->>Zedra: open file",
            ThemePreference::Dark,
        )
        .expect("sequence should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            svg.contains("<text") || svg.contains("tspan"),
            "expected text nodes in mermaid SVG"
        );
    }

    #[test]
    fn renders_flowchart_to_asset_path() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            3,
            "flowchart LR\n  A[Start] --> B[End]",
            ThemePreference::Dark,
        )
        .expect("flowchart should render");
        assert!(diagram.asset_path.starts_with(MERMAID_ASSET_PREFIX));
        assert!(diagram.intrinsic_width > 0.0);
        assert!(diagram.intrinsic_height > 0.0);
        assert!(load_mermaid_svg(&diagram.asset_path).is_some());
    }

    #[test]
    fn dark_quadrant_uses_theme_colors() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(
            12,
            r#"quadrantChart
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
  "Monospace fence only": [0.15, 0.2]"#,
            ThemePreference::Dark,
        )
        .expect("quadrant should render");
        let bytes = load_mermaid_svg(&diagram.asset_path).expect("svg bytes");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("utf-8 svg");
        assert!(
            !svg.contains("#ECECFF"),
            "should not use mermaid default light quadrant bg"
        );
        assert!(
            !svg.contains("#c7c7f1"),
            "should not use mermaid default light quadrant border"
        );
        assert!(
            !svg.contains("#131300"),
            "should not use mermaid default dark quadrant text"
        );
        assert!(
            svg.contains("fill=\"#1a1a1a\""),
            "expected dark quadrant background"
        );
        assert!(
            svg.contains("stroke=\"#505050\""),
            "expected dark quadrant border"
        );
        assert!(
            svg.contains("fill=\"#61afef\""),
            "expected accent-blue point"
        );
    }
}
