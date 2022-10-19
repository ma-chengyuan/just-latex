use std::collections::HashMap;

use anyhow::{Context, Error, Result};
use ouroboros::self_referencing;
use regex::Regex;
use rustybuzz::{shape, Face as ShaperFace, UnicodeBuffer};
use ttf_parser::{Face, GlyphId};
use usvg::{NodeExt, PathBbox};

/// Splits a stream of multiple SVGs (returned by dvisvgm).
pub fn split_svgs(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let mut reader = quick_xml::Reader::from_bytes(bytes);
    let mut cuts = vec![];
    let mut last_pos = 0;
    loop {
        match reader.read_event_unbuffered()? {
            quick_xml::events::Event::Decl(_) => cuts.push(last_pos),
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
        last_pos = reader.buffer_position();
    }
    cuts.push(bytes.len());
    Ok(cuts.windows(2).map(|w| &bytes[w[0]..w[1]]).collect())
}

/// Finds paths and images in an SVG and computes their bboxes.
pub fn paths_to_bboxes(input: &str) -> Result<(usvg::Tree, Vec<PathBbox>)> {
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(input, &options.to_ref())?;
    let mut results = vec![];
    for node in tree.root().descendants() {
        if !node.has_children() {
            if let Some(bbox) = node.calculate_bbox() {
                results.push(bbox);
            }
        }
    }
    Ok((tree, results))
}

/// Finds text elements in an SVG produced by dvisvgm and computes their bboxes.
///
/// Uses a ton of dvisvgm-specific hacks. **ONLY** works for SVGs produced by dvisvgm.
pub fn texts_to_bboxes(input: &str) -> Result<Vec<PathBbox>> {
    let mut reader = quick_xml::Reader::from_str(input);
    let mut font_map = HashMap::new();
    let mut class_map = HashMap::new();
    let mut bboxes = vec![];

    let font_face_regex = Regex::new(
        // Follows the format of dvisvgm's FontWriter.cpp->FontWriter::writeCSSFontFace
        r"@font-face\{font-family:(\w+);src:url\(data:application/x-font-(\w+);base64,([-A-Za-z0-9+/=]+)\) format\('\w+'\);\}",
    )?;
    let font_group_regex = Regex::new(
        // Follows the format of dvisvgm's SVGTree.cpp->SVGTree::appendFontStyles
        r"text\.(f\d+) \{font-family:(\w+);font-size:(\d*\.\d*)px",
    )?;

    struct TextContext {
        x: f64,
        y: f64,
        family: String,
        size: f64,
    }

    let mut text_contexts = vec![];
    loop {
        match reader.read_event_unbuffered()? {
            quick_xml::events::Event::Eof => break,
            quick_xml::events::Event::CData(e) => {
                let inner = e.into_inner();
                let cdata = String::from_utf8_lossy(&inner);
                for capture in font_face_regex.captures_iter(&cdata) {
                    let font_family = capture.get(1).unwrap().as_str();
                    let _font_format = capture.get(2).unwrap().as_str();
                    let font_data = base64::decode(capture.get(3).unwrap().as_str())?;
                    font_map.insert(String::from(font_family), OwnedFace::from_data(font_data)?);
                }
                for capture in font_group_regex.captures_iter(&cdata) {
                    let font_class = capture.get(1).unwrap().as_str();
                    let font_name = capture.get(2).unwrap().as_str();
                    let font_size = capture.get(3).unwrap().as_str().parse::<f64>()?;
                    class_map.insert(
                        String::from(font_class),
                        (String::from(font_name), font_size),
                    );
                }
            }
            quick_xml::events::Event::Start(e) => {
                if e.name() == b"text" {
                    let x = e.try_get_attribute("x")?.context("<text> without x")?;
                    let y = e.try_get_attribute("y")?.context("<text> without x")?;
                    let x_f = String::from_utf8_lossy(&x.value).parse::<f64>()?;
                    let y_f = String::from_utf8_lossy(&y.value).parse::<f64>()?;
                    let (family, size) = if let Some(class) = e.try_get_attribute("class")? {
                        let class_str = String::from_utf8_lossy(&class.value).to_string();
                        class_map
                            .get(&class_str)
                            .context("<text> with unknown class")?
                            .clone()
                    } else {
                        let family = e
                            .try_get_attribute("font-family")?
                            .context("<text> with neither class nor font-family")?;
                        let size = e
                            .try_get_attribute("font-size")?
                            .context("<text> with neither class nor font-family")?;
                        let family_str = String::from_utf8_lossy(&family.value).to_string();
                        let size_str = String::from_utf8_lossy(&size.value);
                        // The font-size here does not contain px. See dvisvg SVGCharHandler.cpp
                        // SVGCharTextHandler::createTextNode()
                        let size_f = size_str.parse::<f64>()?;
                        (family_str, size_f)
                    };
                    text_contexts.push(TextContext {
                        x: x_f,
                        y: y_f,
                        family,
                        size,
                    });
                } else if e.name() == b"tspan" {
                    let last_context = text_contexts.last().context("<tspan> not in <text>")?;
                    text_contexts.push(TextContext {
                        x: if let Some(x) = e.try_get_attribute("x")? {
                            String::from_utf8_lossy(&x.value).parse::<f64>()?
                        } else {
                            last_context.x
                        },
                        y: if let Some(y) = e.try_get_attribute("y")? {
                            String::from_utf8_lossy(&y.value).parse::<f64>()?
                        } else {
                            last_context.y
                        },
                        family: last_context.family.clone(),
                        size: last_context.size,
                    });
                }
            }
            quick_xml::events::Event::End(e) => {
                if e.name() == b"text" || e.name() == b"tspan" {
                    text_contexts.pop();
                }
            }
            quick_xml::events::Event::Text(e) => {
                if let Some(TextContext { x, y, family, size }) = text_contexts.last() {
                    let inner = e.into_inner();
                    let text = String::from_utf8_lossy(&inner);
                    if text.len() > 0 {
                        let face = font_map
                            .get(family)
                            .with_context(|| format!("unknown family {family}"))?;
                        let scale = size / face.borrow_shaper_face().units_per_em() as f64;
                        let mut buffer = UnicodeBuffer::new();
                        let features = vec![];
                        buffer.push_str(&text);
                        let buffer = shape(face.borrow_shaper_face(), &features, buffer);
                        let mut x = *x;
                        let mut y = *y;
                        let mut x_min = f64::MAX;
                        let mut x_max = f64::MIN;
                        let mut y_min = f64::MAX;
                        let mut y_max = f64::MIN;
                        for (info, pos) in buffer.glyph_infos().iter().zip(buffer.glyph_positions())
                        {
                            let bbox = face
                                .borrow_face()
                                .glyph_bounding_box(GlyphId(info.glyph_id as u16))
                                .expect("unknown glyph id in shaper output");
                            let g_x_min = (pos.x_offset as f64 + bbox.x_min as f64) * scale;
                            let g_x_max = (pos.x_offset as f64 + bbox.x_max as f64) * scale;
                            let g_y_min = (pos.y_offset as f64 + bbox.y_min as f64) * scale;
                            let g_y_max = (pos.y_offset as f64 + bbox.y_max as f64) * scale;
                            x_min = x_min.min(x + g_x_min);
                            x_max = x_max.max(x + g_x_max);
                            y_min = y_min.min(y - g_y_max);
                            y_max = y_max.max(y - g_y_min);
                            /*
                            println!(
                                "  {} {:.2}--{:.2}({:.2}) {:.2}--{:.2}({:.2})",
                                info.glyph_id, x_min, x_max, x_max - x_min, y_min, y_max, y_max - y_min
                            );
                            */
                            /*
                            eprintln!(
                                r#"<rect x="{x_min}" y="{y_min}" width="{}" height="{}" style="fill:none;stroke:red;"/>"#,
                                x_max - x_min,
                                y_max - y_min
                            );
                            */
                            x += pos.x_advance as f64 * scale;
                            y += pos.y_advance as f64 * scale;
                        }
                        bboxes.push(
                            PathBbox::new(x_min, y_min, x_max - x_min, y_max - y_min).unwrap(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(bboxes)
}

#[self_referencing]
struct OwnedFace {
    data: Vec<u8>,
    #[borrows(data)]
    #[covariant]
    face: Face<'this>,
    #[borrows(data)]
    #[covariant]
    shaper_face: ShaperFace<'this>,
}

impl OwnedFace {
    fn from_data(data: Vec<u8>) -> Result<Self> {
        OwnedFaceTryBuilder {
            data,
            face_builder: |data: &Vec<u8>| Face::parse(data, 0).map_err(Error::new),
            shaper_face_builder: |data: &Vec<u8>| {
                ShaperFace::from_slice(data, 0).ok_or_else(|| Error::msg("cannot parse face"))
            },
        }
        .try_build()
    }
}

/// Given a slice of bounding boxes and a y range, compute the x range that exactly covers all
/// bounding boxes which have non-empty intersection with the y range. There is a tolerance term
/// for robustness, because dvisvgm and synctex aren't always very accurate.
pub fn x_range_for_y_range(
    bboxes: &[PathBbox],
    y_min: f64,
    y_max: f64,
    tol: f64,
    margin: f64,
) -> Option<(f64, f64)> {
    let mut x_min = f64::MAX;
    let mut x_max = f64::MIN;
    let y_min = y_min - tol;
    let y_max = y_max + tol;
    for bbox in bboxes {
        if y_min.max(bbox.top()) <= y_max.min(bbox.bottom()) {
            x_min = x_min.min(bbox.left());
            x_max = x_max.max(bbox.right());
        }
    }
    if x_min == f64::MAX {
        None
    } else {
        Some((x_min - margin, x_max + margin))
    }
}

// TODO: perhaps merge the function below with the function above, to save one full traversal of
// bboxes.
pub fn refine_y_range(
    bboxes: &[PathBbox],
    y_min: f64,
    y_max: f64,
    tol: f64,
    margin: f64,
) -> (f64, f64) {
    let mut new_y_min = f64::MAX;
    let mut new_y_max = f64::MIN;
    let y_min = y_min - tol;
    let y_max = y_max + tol;
    for bbox in bboxes {
        // if y_min <= bbox.top() && bbox.bottom() <= y_max {
        if y_min.max(bbox.top()) <= y_max.min(bbox.bottom()) {
            new_y_min = new_y_min.min(bbox.top());
            new_y_max = new_y_max.max(bbox.bottom());
        }
    }
    if new_y_min == f64::MAX {
        (y_min + tol - margin, y_max - tol + margin)
    } else {
        (new_y_min - margin, new_y_max + margin)
    }
}
