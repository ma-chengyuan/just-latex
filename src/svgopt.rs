//! SVG optimization!
//!
//! For PDF inputs, dvisvgm converts all texts to paths (it has to do that for some glyphs in TeX
//! fonts are not accessible from Unicode code points). When there are multiple occurence of the
//! same glyph and its shape is complex, converting each of them into a path bloats the SVG.
//!
//! This module aims to identify similar paths, define them once in a `<defs/>` section and replace
//! all their references with `<use/>`. This is easier said than done, because floating point
//! numbers are inexact and what we got is their value rounded to limited-precision decimals (in
//! the textual output of dvisvgm). Hence, all similarity checks have to be done in a fuzzy manner.
//! And we have thousands of paths. A naive implementation would take O(nm) time (where n is the
//! number of paths and n is the size of the SVG), which is too slow. This module takes a Trie-like
//! approach and reduced the overall time complexity to O(nlogm).
//!
//! As for the outcomes. Empirical testing shows that the optimized SVG can be as small as 20% of
//! the original SVG (uncompressed). However, when LZMA compression are later applied, there is
//! no significant difference between the size of the compressed files. The optimized SVG may even
//! come out larger when compressed. Nevertheless, both the compression and decompression time are
//! greatly reduced -- this means the web page will load faster.

use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
    time::Instant,
};

use anyhow::Result;
use ordered_float::OrderedFloat;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use usvg::{NodeKind, Paint, Path, PathSegment, Tree, XmlOptions};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum PathCommand {
    MoveTo,
    LineTo,
    CurveTo,
    ClosePath,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum PathFingerprintElement {
    Command(PathCommand),
    Coord(OrderedFloat<f64>),
}

#[derive(Default)]
struct PathTree {
    cmd_nodes: HashMap<PathCommand, Box<PathTree>>,
    coord_nodes: BTreeMap<OrderedFloat<f64>, Box<PathTree>>,
    paths: Vec<Path>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PathFingerprint {
    elems: Vec<PathFingerprintElement>,
    shift: (OrderedFloat<f64>, OrderedFloat<f64>),
}

impl PathTree {
    pub fn find_similar(&self, fingerprint: &PathFingerprint, eps: f64) -> &[Path] {
        // DFS on the tree.
        let mut q = vec![(self, &fingerprint.elems[..])];
        while let Some((node, rest)) = q.pop() {
            if let Some(first) = rest.first() {
                match first {
                    PathFingerprintElement::Command(cmd) => {
                        if let Some(next) = node.cmd_nodes.get(cmd) {
                            q.push((next, &rest[1..]));
                        }
                    }
                    PathFingerprintElement::Coord(x) => {
                        q.extend(
                            node.coord_nodes
                                .range((x - eps)..=(x + eps))
                                .map(|(_, next)| (next.as_ref(), &rest[1..])),
                        );
                    }
                }
            } else {
                return &node.paths;
            }
        }
        &[]
    }

    pub fn insert(&mut self, fingerprint: &PathFingerprint, path: &Path) {
        self.insert_slice(&fingerprint.elems, path);
    }

    fn insert_slice(&mut self, elems: &[PathFingerprintElement], path: &Path) {
        if let Some(first) = elems.first() {
            match first {
                PathFingerprintElement::Command(cmd) => {
                    self.cmd_nodes.entry(*cmd).or_insert_with(Default::default)
                }
                PathFingerprintElement::Coord(x) => {
                    self.coord_nodes.entry(*x).or_insert_with(Default::default)
                }
            }
            .insert_slice(&elems[1..], path)
        } else {
            self.paths.push(path.clone());
        }
    }
}

impl PathFingerprint {
    fn new(path: &Path) -> Self {
        let mut shift: Option<(f64, f64)> = None;
        let mut elems = vec![];
        for segment in path.data.0.iter() {
            match segment {
                PathSegment::MoveTo { x, y } => {
                    let (x, y) = path.transform.apply(*x, *y);
                    let (dx, dy) = *shift.get_or_insert((x, y));
                    elems.push(PathFingerprintElement::Command(PathCommand::MoveTo));
                    elems.push(PathFingerprintElement::Coord((x - dx).into()));
                    elems.push(PathFingerprintElement::Coord((y - dy).into()));
                }
                PathSegment::LineTo { x, y } => {
                    let (x, y) = path.transform.apply(*x, *y);
                    let (dx, dy) = *shift.get_or_insert((x, y));
                    elems.push(PathFingerprintElement::Command(PathCommand::LineTo));
                    elems.push(PathFingerprintElement::Coord((x - dx).into()));
                    elems.push(PathFingerprintElement::Coord((y - dy).into()));
                }
                #[rustfmt::skip]
                PathSegment::CurveTo { x1, y1, x2, y2, x, y } => {
                    let (x1, y1) = path.transform.apply(*x1, *y1);
                    let (x2, y2) = path.transform.apply(*x2, *y2);
                    let (x, y) = path.transform.apply(*x, *y);
                    let (dx, dy) = *shift.get_or_insert((x, y));
                    elems.push(PathFingerprintElement::Command(PathCommand::CurveTo));
                    elems.push(PathFingerprintElement::Coord((x1 - dx).into()));
                    elems.push(PathFingerprintElement::Coord((y1 - dy).into()));
                    elems.push(PathFingerprintElement::Coord((x2 - dx).into()));
                    elems.push(PathFingerprintElement::Coord((y2 - dy).into()));
                    elems.push(PathFingerprintElement::Coord((x - dx).into()));
                    elems.push(PathFingerprintElement::Coord((y - dy).into()));
                }
                PathSegment::ClosePath => {
                    elems.push(PathFingerprintElement::Command(PathCommand::ClosePath));
                }
            }
        }
        Self {
            elems,
            shift: shift
                .map(|(a, b)| (OrderedFloat(a), OrderedFloat(b)))
                .unwrap(),
        }
    }
}

fn same_style(a: &Path, b: &Path, eps: f64) -> bool {
    fn same_paint(a: &Paint, b: &Paint) -> bool {
        match (a, b) {
            (Paint::Color(a), Paint::Color(b)) => a == b,
            (Paint::Link(a), Paint::Link(b)) => a == b,
            _ => false,
        }
    }

    let same_stroke = match (&a.stroke, &b.stroke) {
        (Some(a), Some(b)) => {
            let same_dashoffset = (a.dashoffset - b.dashoffset).abs() <= eps as f32;
            let same_dasharray = match (&a.dasharray, &b.dasharray) {
                (Some(a), Some(b)) => a.iter().zip(b.iter()).all(|(a, b)| (a - b).abs() <= eps),
                (None, None) => true,
                _ => false,
            };
            same_paint(&a.paint, &b.paint)
                && same_dashoffset
                && same_dasharray
                && a.linecap == b.linecap
                && a.linejoin == b.linejoin
                && a.miterlimit == b.miterlimit
                && a.opacity == b.opacity
        }
        (None, None) => true,
        _ => false,
    };

    let same_fill = match (&a.fill, &b.fill) {
        (Some(a), Some(b)) => {
            same_paint(&a.paint, &b.paint) && a.opacity == b.opacity && a.rule == b.rule
        }
        (None, None) => true,
        _ => false,
    };

    same_stroke && same_fill && a.rendering_mode == b.rendering_mode && a.visibility == b.visibility
}

pub fn optimize(tree: &Tree, eps: f64) -> Result<Vec<u8>> {
    let start = Instant::now();
    let mut path_tree = PathTree::default();
    let mut count = 0usize;
    let mut total = 0usize;

    enum State {
        Standalone,
        Referred,
        Referring(usize),
    }

    let delim = '|';
    let mut states: Vec<(State, PathFingerprint)> = vec![];

    for mut node in tree.root().descendants() {
        if !node.has_children() {
            if let NodeKind::Path(p) = &mut *node.borrow_mut() {
                let id = total;
                // Temporarily prefix path ids with their indices, so we can identify them in the
                // SVG output. They will be stripped off by then.
                p.id = format!("{}{}{}", id, delim, p.id);
                let fingerprint = PathFingerprint::new(p);
                if let Some(similar) = path_tree
                    .find_similar(&fingerprint, eps)
                    .iter()
                    .find(|s| same_style(s, p, eps))
                {
                    let p_id = similar.id[..similar.id.find(delim).unwrap()].parse::<usize>()?;
                    states[p_id].0 = State::Referred;
                    states.push((State::Referring(p_id), fingerprint));
                    count += 1;
                } else {
                    path_tree.insert(&fingerprint, p);
                    states.push((State::Standalone, fingerprint));
                }
                total += 1;
            }
        }
    }

    let opt = XmlOptions::default();
    let unoptimized = tree.to_string(&opt);
    let mut reader = quick_xml::Reader::from_str(&unoptimized);
    let mut writer = quick_xml::Writer::new(Cursor::new(vec![]));

    // Surprisingly the XML specs says much of the Unicode characters are valid ids, so let' s use
    // them to avoid possible conflicts.
    let format_def_id = |id: usize| format!("ⱼₗ{}", id);
    let format_use_id = |id: usize| format!("#ⱼₗ{}", id);

    let mut defs: Vec<(usize, BytesStart)> = vec![];
    loop {
        match reader.read_event_unbuffered()? {
            Event::End(e) => {
                if e.name() == b"svg" {
                    writer.write_event(Event::Start(BytesStart::owned_name("defs")))?;
                    let mut dummy = vec![];
                    std::mem::swap(&mut defs, &mut dummy);
                    for (id, start) in dummy {
                        let mut g = BytesStart::owned_name("g");
                        g.push_attribute(("id", format_def_id(id).as_str()));
                        writer.write_event(Event::Start(g))?;
                        writer.write_event(Event::Empty(start))?;
                        writer.write_event(Event::End(BytesEnd::borrowed(b"g")))?;
                    }
                    writer.write_event(Event::End(BytesEnd::borrowed(b"defs")))?;
                    writer.write_event(Event::End(e))?;
                }
            }
            Event::Empty(e) => {
                if e.name() == b"path" {
                    let id = e.try_get_attribute("id")?.unwrap();
                    let id_str = String::from_utf8_lossy(&id.value);
                    let delim_pos = id_str.find(delim).unwrap();
                    let id = id_str[..delim_pos].parse::<usize>()?;

                    let remove_id_prefix = || -> Result<BytesStart> {
                        let mut new_e = BytesStart::owned_name("path");
                        for attr in e.attributes() {
                            let attr = attr?;
                            if attr.key == b"id" {
                                new_e.push_attribute(("id", &id_str[delim_pos + 1..]));
                            } else {
                                new_e.push_attribute(attr);
                            }
                        }
                        Ok(new_e)
                    };

                    match &states[id] {
                        (State::Standalone, _) => {
                            writer.write_event(Event::Empty(remove_id_prefix()?))?;
                        }
                        (State::Referring(r_id), fp) => {
                            let target_shift = states[*r_id].1.shift;
                            let shift = (
                                format!("{:.3}", fp.shift.0 - target_shift.0),
                                format!("{:.3}", fp.shift.1 - target_shift.1),
                            );
                            let mut new_use = BytesStart::owned_name("use");
                            new_use.push_attribute(("x", shift.0.as_str()));
                            new_use.push_attribute(("y", shift.1.as_str()));
                            new_use.push_attribute(("href", format_use_id(*r_id).as_str()));
                            writer.write_event(Event::Empty(new_use))?;
                        }
                        (State::Referred, _) => {
                            let mut new_use = BytesStart::owned_name("use");
                            new_use.push_attribute(("x", "0"));
                            new_use.push_attribute(("y", "0"));
                            new_use.push_attribute(("href", format_use_id(id).as_str()));
                            writer.write_event(Event::Empty(new_use))?;
                            defs.push((id, remove_id_prefix()?));
                        }
                    }
                } else {
                    writer.write_event(Event::Empty(e))?;
                }
            }
            Event::Eof => break,
            e => writer.write_event(e)?,
        }
    }

    eprintln!(
        "SVG optimizer found {}/{} similar paths in {}s",
        count,
        total,
        start.elapsed().as_secs_f64()
    );
    Ok(writer.into_inner().into_inner())
}
