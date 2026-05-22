use mermaid_rs_renderer::{RenderOptions, render_with_options};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

const MERMAID_ASSET_PREFIX: &str = "mermaid/";

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

pub fn render_mermaid_diagram(block_ix: usize, source: &str) -> Option<MermaidDiagram> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }

    let mut options = RenderOptions::default();
    let mut theme = options.theme.clone();
    theme.background = hex_color(crate::theme::BG_CARD);
    theme.text_color = hex_color(crate::theme::TEXT_PRIMARY);
    theme.primary_color = hex_color(crate::theme::BG_CARD);
    theme.primary_text_color = hex_color(crate::theme::TEXT_PRIMARY);
    theme.primary_border_color = hex_color(crate::theme::BORDER_DEFAULT);
    theme.line_color = hex_color(crate::theme::BORDER_DEFAULT);
    theme.secondary_color = hex_color(crate::theme::BG_CARD);
    theme.tertiary_color = hex_color(crate::theme::BORDER_SUBTLE);
    theme.cluster_background = hex_color(crate::theme::BG_CARD);
    theme.cluster_border = hex_color(crate::theme::BORDER_DEFAULT);
    options.theme = theme;

    let svg = render_with_options(source, options).ok()?;
    let (intrinsic_width, intrinsic_height) = svg_intrinsic_size(&svg)?;
    let asset_path = mermaid_asset_path(block_ix, source);
    store_mermaid_svg(asset_path.clone(), Arc::from(svg.into_bytes()));

    Some(MermaidDiagram {
        asset_path,
        intrinsic_width,
        intrinsic_height,
    })
}

fn hex_color(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xffffff)
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
    fn renders_flowchart_to_asset_path() {
        clear_mermaid_svg_cache();
        let diagram = render_mermaid_diagram(3, "flowchart LR\n  A[Start] --> B[End]")
            .expect("flowchart should render");
        assert!(diagram.asset_path.starts_with(MERMAID_ASSET_PREFIX));
        assert!(diagram.intrinsic_width > 0.0);
        assert!(diagram.intrinsic_height > 0.0);
        assert!(load_mermaid_svg(&diagram.asset_path).is_some());
    }
}
