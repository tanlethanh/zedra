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
        Self {
            canvas: ui.bg_card,
            node_fill: ui.border_subtle,
            node_fill_alt,
            node_border: ui.border_default,
            ink: ui.text_primary,
            ink_muted: ui.text_secondary,
            edge_stroke: ui.accent_blue,
            note_fill,
        }
    }
}

/// In-memory SVG assets for dynamically rendered Mermaid diagrams (`img()` embedded paths).
static MERMAID_SVG_CACHE: OnceLock<Mutex<HashMap<String, Arc<[u8]>>>> = OnceLock::new();

pub fn mermaid_asset_path(block_ix: usize, source: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!(
        "{MERMAID_ASSET_PREFIX}{block_ix}-{:x}.svg",
        hasher.finish()
    )
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

    let svg = render_with_options(source, mermaid_render_options(preference)).ok()?;
    let (intrinsic_width, intrinsic_height) = svg_intrinsic_size(&svg)?;
    let asset_path = mermaid_asset_path(block_ix, source);
    store_mermaid_svg(asset_path.clone(), Arc::from(svg.into_bytes()));

    Some(MermaidDiagram {
        asset_path,
        intrinsic_width,
        intrinsic_height,
    })
}

pub(crate) fn mermaid_render_options(preference: ThemePreference) -> RenderOptions {
    let mut options = RenderOptions::default();
    options.theme = mermaid_theme_for_preference(preference);
    configure_flowchart_layout(&mut options.layout);
    options
}

fn configure_flowchart_layout(layout: &mut LayoutConfig) {
    // Prefer straight vertical/horizontal segments over zig-zag grid detours.
    layout.flowchart.routing.turn_penalty = 2.0;
    layout.flowchart.routing.grid_cell = 12.0;
    layout.flowchart.objective.edge_relax_passes = 3;
}

pub(crate) fn mermaid_theme_for_preference(preference: ThemePreference) -> Theme {
    let palette = MermaidPalette::for_preference(preference);
    let canvas = hex_color(palette.canvas);
    let node = hex_color(palette.node_fill);
    let node_alt = hex_color(palette.node_fill_alt);
    let node_border = hex_color(palette.node_border);
    let ink = hex_color(palette.ink);
    let ink_muted = hex_color(palette.ink_muted);
    let edge = hex_color(palette.edge_stroke);
    let note = hex_color(palette.note_fill);

    let mut theme = Theme::modern();
    theme.font_family = "IBM Plex Sans".to_string();
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

    theme.pie_colors = pie_colors_for_preference(preference);
    theme.pie_title_text_color = ink.clone();
    theme.pie_section_text_color = ink.clone();
    theme.pie_legend_text_color = ink.clone();
    theme.pie_stroke_color = node_border.clone();
    theme.pie_outer_stroke_color = node_border.clone();

    theme
}

fn hex_color(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xffffff)
}

fn pie_colors_for_preference(preference: ThemePreference) -> [String; 12] {
    let ui = match preference {
        ThemePreference::Dark => ThemePalette::dark(),
        ThemePreference::Light => ThemePalette::light(),
    };
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
        let theme = mermaid_theme_for_preference(ThemePreference::Dark);
        assert_eq!(theme.background, "#131313");
        assert_eq!(theme.primary_color, "#1a1a1a");
        assert_eq!(theme.primary_text_color, "#ffffff");
        assert_eq!(theme.pie_legend_text_color, "#ffffff");
        assert_eq!(theme.text_color, "#ffffff");
        assert_eq!(theme.line_color, "#61afef");
    }

    #[test]
    fn diagrams_md_blocks_parse_and_render() {
        use mermaid_rs_renderer::parse_mermaid;
        use std::fs;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/markdown-mermaid/diagrams.md");
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
            render_mermaid_diagram(0, source, ThemePreference::Dark).unwrap_or_else(|| {
                panic!("render failed for [{section}]\n---\n{source}\n---")
            });
        }
    }

    #[test]
    fn flowchart_edges_use_accent_stroke() {
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
            svg.contains("stroke=\"#61afef\""),
            "expected accent edge stroke"
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
}
