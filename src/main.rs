use anyhow::{bail, Context, Result};
use bytesize::ByteSize;
use indoc::formatdoc;
use serde_json::{json, Value};
use std::{
    borrow::Cow,
    collections::{hash_map::DefaultHasher, BTreeMap, HashSet},
    fs::File,
    hash::{Hash, Hasher},
    io::{stdin, stdout, Cursor, Read, Write},
    ops::Range,
    path::Path,
    process::Command,
    str::FromStr,
    time::Instant,
    vec,
};
use tempfile::TempDir;
use usvg::{NodeExt, PathBbox};
use xz2::{read::XzEncoder, stream::LzmaOptions};

use crate::synctex::Scanner;
use crate::{config::Config, svgopt::optimize};

mod config;
mod svgopt;
mod synctex;

fn main() -> Result<()> {
    let config = Config::load()?;
    let mut buffer = String::new();
    let _ = stdin().read_to_string(&mut buffer)?;

    let mut tree = Value::from_str(&buffer)?;
    FragmentRenderer::new(config).render_with_latex(&mut tree)?;

    let output = serde_json::to_vec(&tree)?;
    stdout().write_all(&output)?;
    Ok(())
}

#[derive(Debug)]
struct FragmentRenderer<'a> {
    config: Config,
    fragments: Vec<Fragment<'a>>,
}

#[derive(Debug)]
struct Fragment<'a> {
    ty: FragmentType,
    src: String,
    refs: Vec<FragmentNodeRef<'a>>,
}

#[derive(Debug)]
enum FragmentNodeRef<'a> {
    Inline(&'a mut Value),
    Block(&'a mut Value),
}

#[derive(Debug)]
enum FragmentType {
    /// For ordinary inline maths.
    InlineMath,
    /// For display maths.
    DisplayMath,
    /// These will be included in the .tex file without being surrounded by "{}".
    RawBlock,
    /// For display maths starting with %dontshow. They are included in the tex files but not shown.
    /// Use them for macro definitions.
    DontShow,
}

impl<'a> FragmentRenderer<'a> {
    fn new(config: Config) -> Self {
        Self {
            config,
            fragments: vec![],
        }
    }

    fn add_fragment(&mut self, ty: FragmentType, src: &str, node_ref: FragmentNodeRef<'a>) {
        match ty {
            // Inline fragments are often duplicates of previous ones encountered.
            // Caveat: if inline fragments contain expansions of macro with side effect (which is
            // rather unlikely), then this could cause trouble!
            FragmentType::InlineMath => {
                let src = src.trim();
                for item in self.fragments.iter_mut() {
                    match item.ty {
                        FragmentType::InlineMath if item.src == src => {
                            item.refs.push(node_ref);
                            return;
                        }
                        _ => continue,
                    }
                }
                self.fragments.push(Fragment {
                    ty,
                    src: src.into(),
                    refs: vec![node_ref],
                });
            }
            _ => {
                self.fragments.push(Fragment {
                    ty,
                    src: src.trim().into(),
                    refs: vec![node_ref],
                });
            }
        }
    }

    fn generate_latex_with_line_mappings(&self) -> (String, Vec<Range<usize>>) {
        let mut lines: Vec<Range<usize>> = vec![];
        let mut output = String::new();
        let preamble_trimmed = self.config.preamble.trim_end();
        output.push_str(preamble_trimmed);
        output.push('\n');
        let mut current_line = preamble_trimmed.lines().count() + 1;
        for item in self.fragments.iter() {
            let template = match item.ty {
                FragmentType::InlineMath => &self.config.template.inline_math,
                FragmentType::DisplayMath => &self.config.template.display_math,
                FragmentType::RawBlock | FragmentType::DontShow => {
                    &self.config.template.placeholder
                }
            };
            let expanded = template.replace(&self.config.template.placeholder, &item.src);
            let expanded = expanded.trim_end();
            let start_line = current_line;
            output.push_str(expanded);
            current_line += expanded.lines().count();
            lines.push(start_line..current_line);
            output.push_str("\n\n");
            current_line += 1;
        }
        output.push_str(&self.config.postamble);
        (output, lines)
    }

    /// Scans and modifies the tree in-place, replacing all inline and display maths with rendered
    /// SVGs.
    pub fn render_with_latex(mut self, tree: &'a mut Value) -> Result<()> {
        let final_node = self.walk_and_create_final_node(tree)?;

        if self.fragments.is_empty() {
            // Make the final node a dummy.
            *final_node = json!({"t": "RawBlock", "c": ["html", ""]});
            return Ok(());
        }

        // dvisvgm does very spurious scaling to the output svg even when no magnification arguments
        // are passed. Besides the viewboxes are very weird.
        // x_svg = x_tex * 0.996264;
        // y_svg = y_tex * 0.996264 - page_height;
        const TEX2SVG_SCALING: f64 = 72.0 / 72.27;

        let (source_str, lines) = self.generate_latex_with_line_mappings();
        let working_dir = match self.config.output_folder {
            Some(_) => None,
            None => Some(TempDir::new()?),
        };
        let working_path = match &working_dir {
            Some(working_dir) => working_dir.path().to_path_buf(),
            None => Path::new(&self.config.output_folder.unwrap()).to_path_buf(),
        }
        .canonicalize()?;
        let source_path = working_path.join("source.tex");

        // eprintln!("{}", source_str);
        {
            let mut source = File::create(&source_path)?;
            source.write_all(source_str.as_bytes())?;
        }
        let pdf_path = working_path.join("source.pdf");
        let latex_command = Command::new(self.config.latex)
            .args([
                "-synctex=-1",
                "-interaction=nonstopmode",
                source_path.to_str().unwrap(),
            ])
            .current_dir(&working_path)
            .output()?;
        if !latex_command.status.success() {
            eprintln!(
                "latex error: {}",
                String::from_utf8(latex_command.stdout).unwrap()
            );
            bail!("fail to run latex");
        }

        // Right now we assume everything fits in one page so there's only one svg generated.
        // This requires a page to be REALLY long/high. TeX can handle at most 65536pt -- I assume
        // this is enough as long as we are not writing a book.
        let dvisvgm_command = Command::new(self.config.dvisvgm)
            .args(["-s", "-R", "-P", "-p1-", pdf_path.to_str().unwrap()])
            .current_dir(&working_path)
            .output()?;
        if !dvisvgm_command.status.success() {
            bail!("fail to run dvisvgm");
        }
        let options = usvg::Options::default();
        // Split svgs because we might have multiple pages.
        let svg_data = split_svgs(&dvisvgm_command.stdout)?;
        let svgs = svg_data
            .iter()
            .map(|&svg| usvg::Tree::from_data(svg, &options.to_ref()))
            .collect::<Result<Vec<_>, _>>()?;

        // A unique class name for each svg is important because HTMLs from multiple posts
        // may be put together in the home page of a blog. Then the decompressing code of each page
        // starts a race, each trying to modify every fragment image.
        let svg_class_names = svg_data
            .iter()
            .map(|svg| {
                let mut hasher = DefaultHasher::new();
                svg.hash(&mut hasher);
                let hash = hasher.finish();
                format!("jl-{}", base64::encode(hash.to_be_bytes()))
            })
            .collect::<Vec<_>>();

        let bboxes = svgs
            .iter()
            .map(|svg| svg_to_bboxes(svg.root()))
            .collect::<Vec<_>>();

        let scanner = Scanner::new(pdf_path, &working_path);
        let mut seen_boxes = HashSet::new();

        for (item, line_range) in self.fragments.iter_mut().zip(lines) {
            if let FragmentType::DontShow = item.ty {
                // Skip dont shows.
                for node in item.refs.iter_mut() {
                    match node {
                        FragmentNodeRef::Inline(node) => {
                            **node = json!({"t": "RawInline", "c": ["html", ""]})
                        }
                        FragmentNodeRef::Block(node) => {
                            **node = json!({"t": "RawBlock", "c": ["html", ""]});
                        }
                    }
                }
                continue;
            }

            #[derive(Clone, Debug)]
            struct Region {
                x_range: (f64, f64),
                y_range: (f64, f64),
                baseline: f64,
                baseline_width: f64,
            }

            let mut regions: BTreeMap<u32, Region> = BTreeMap::new();

            for line in line_range {
                for tb in scanner.query(line) {
                    let area = tb.width * (tb.height + tb.depth);
                    if area.into_inner() <= 1e-6 {
                        // Skip zero-area boxes. They may be generated by the TeX page breaker and
                        // do not actually correspond to anything in our source file. Also they
                        // wouldn't contribute to updating the region of the page anyways.
                        continue;
                    }
                    if seen_boxes.contains(&tb) {
                        // Continue if we have seen this box -- then probably that's SyncTeX's
                        // fault
                        continue;
                    }
                    seen_boxes.insert(tb.clone());

                    let (x_low, x_high) = (tb.h.into_inner(), (tb.h + tb.width).into_inner());
                    let (y_low, y_high) = (
                        (tb.v - tb.height).into_inner(),
                        (tb.v + tb.depth).into_inner(),
                    );
                    regions
                        .entry(tb.page)
                        .and_modify(|r| {
                            r.x_range = (r.x_range.0.min(x_low), r.x_range.1.max(x_high));
                            r.y_range = (r.y_range.0.min(y_low), r.y_range.1.max(y_high));
                            if tb.width.into_inner() > r.baseline_width {
                                r.baseline_width = tb.width.into_inner();
                                r.baseline = tb.v.into_inner();
                            }
                        })
                        .or_insert_with(|| Region {
                            x_range: (x_low, x_high),
                            y_range: (y_low, y_high),
                            baseline: tb.v.into(),
                            baseline_width: tb.width.into(),
                        });
                }
            }

            if regions.is_empty() {
                bail!("no boxes for {}", item.src);
            }
            if matches!(item.ty, FragmentType::InlineMath) && regions.len() > 1 {
                bail!(
                    "inline fragments '{}' spans multiple pages {:?} (did you disable page numbering?)",
                    item.src,
                    regions.keys().collect::<Vec<_>>()
                );
            }

            let mut imgs = vec![];
            for (
                page,
                Region {
                    mut x_range,
                    mut y_range,
                    mut baseline,
                    ..
                },
            ) in regions.into_iter()
            {
                let svg_idx = page as usize - 1;
                let y_base = svgs[svg_idx].svg_node().view_box.rect.top();
                // Convert everything from TeX coordinates to SVG coordinates.
                y_range = (
                    y_range.0 * TEX2SVG_SCALING + y_base,
                    y_range.1 * TEX2SVG_SCALING + y_base,
                );
                x_range = x_range_for_y_range(
                    &bboxes[svg_idx],
                    y_range.0,
                    y_range.1,
                    self.config.y_range_tol,
                    self.config.x_range_margin,
                )
                .unwrap_or((x_range.0 * TEX2SVG_SCALING, x_range.1 * TEX2SVG_SCALING));
                baseline = baseline * TEX2SVG_SCALING + y_base;

                if let FragmentType::DisplayMath | FragmentType::RawBlock = item.ty {
                    y_range = refine_y_range(
                        &bboxes[svg_idx],
                        y_range.0,
                        y_range.1,
                        self.config.y_range_tol,
                        self.config.y_range_margin,
                    );
                }

                let depth = match item.ty {
                    FragmentType::InlineMath => y_range.1 - baseline,
                    FragmentType::DisplayMath | FragmentType::RawBlock => 0.0,
                    FragmentType::DontShow => unreachable!(),
                };
                imgs.push(formatdoc!(
                    r##"<img src="#svgView(viewBox({x:.2},{y:.2},{width:.2},{height:.2}))"
                         class="{class_name}" alt = "{alt}"
                         style="width:{width:.2}pt;height:{height:.2}pt;
                         top:{depth:.2}pt;position:relative;display:inline;margin-bottom:0pt;">"##,
                    x = x_range.0,
                    y = y_range.0,
                    class_name = svg_class_names[svg_idx],
                    width = x_range.1 - x_range.0,
                    height = y_range.1 - y_range.0,
                    depth = depth - self.config.baseline_rise,
                    alt = html_escape::encode_text(&item.src),
                ));
            }
            let html = match item.ty {
                FragmentType::InlineMath => imgs.join(""),
                FragmentType::DisplayMath | FragmentType::RawBlock => {
                    format!(r#"<p style="text-align:center;">{}</p>"#, imgs.join("<br>"))
                }
                FragmentType::DontShow => unreachable!(),
            };
            for node in item.refs.iter_mut() {
                match node {
                    FragmentNodeRef::Inline(node) => {
                        **node = json!({"t": "RawInline", "c": ["html", &html]});
                    }
                    FragmentNodeRef::Block(node) => {
                        **node = json!({"t": "RawBlock", "c": ["html", &html]});
                    }
                }
            }
        }

        let lzma_options = LzmaOptions::new_preset(9)?;
        let mut decompress_script = String::new();
        let svg_data = if self.config.optimizer.enabled {
            svgs.iter()
                .map(|tree| -> Result<Cow<[u8]>> {
                    Ok(Cow::Owned(optimize(tree, self.config.optimizer.eps)?))
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            svg_data.iter().map(|data| Cow::Borrowed(*data)).collect()
        };
        for (i, (svg, class_name)) in svg_data.into_iter().zip(svg_class_names).enumerate() {
            let start = Instant::now();
            let original_size = svg.len();
            let mut svg_compressor = XzEncoder::new_stream(
                Cursor::new(svg),
                xz2::stream::Stream::new_lzma_encoder(&lzma_options)?,
            );
            let mut svg_compressed = vec![];
            svg_compressor.read_to_end(&mut svg_compressed)?;
            let svg_encoded = base64::encode(svg_compressed);
            decompress_script.push_str(&formatdoc!(r##"
                console.time("decompress_{page}");
                LZMA.decompress(Uint8Array.from(atob("{svg}"), function(c) {{ return c.charCodeAt(0); }}), 
                    function(result, error) {{
                        console.timeEnd("decompress_{page}");
                        var svgUrl = URL.createObjectURL(new Blob([result], {{type: "image/svg+xml"}}));
                        var imgs = document.getElementsByClassName("{class_name}");
                        for (var i = 0; i < imgs.length; i++) {{
                            var hashPos = imgs[i].src.indexOf("#");
                            if (hashPos != -1)
                                imgs[i].src = svgUrl + imgs[i].src.substring(hashPos);
                        }}
                    }}, 
                    function(p) {{}}
                );
            "##,
                page = i + 1, svg = svg_encoded, class_name = class_name
            ));

            eprintln!(
                "SVG for page {} compressed from {} down to {} (base64 encoded) in {}s",
                i + 1,
                ByteSize::b(original_size as u64),
                ByteSize::b(svg_encoded.len() as u64),
                start.elapsed().as_secs_f64()
            );
        }

        let final_code = format!(
            r"{}<script>{}</script>",
            self.config.lzma_script, decompress_script
        );
        *final_node = json!({
            "t": "RawBlock",
            "c": [
                "html",
                final_code,
            ]
        });
        Ok(())
    }

    /// Walks the tree and look for math nodes. Also creates and returns the reference to an empty
    /// final node, which we will modify later. Due to the borrow checker this is the only place we
    /// can add stuff to the tree. If we just call self.walk_blocks(&mut tree["blocks"], "Document")
    /// in render_with_latex() and try to modify tree["blocks"] afterwards, the borrow checker will
    /// complain.
    fn walk_and_create_final_node(&mut self, tree: &'a mut Value) -> Result<&'a mut Value> {
        let blocks = tree["blocks"]
            .as_array_mut()
            .context("reading [blocks] of the Document")?;
        let last_idx = blocks.len();
        blocks.push(json!({}));
        let mut ret = None;
        for (i, block) in blocks.iter_mut().enumerate() {
            if i == last_idx {
                ret = Some(block);
            } else {
                self.walk_block(block)?;
            }
        }
        Ok(ret.unwrap())
    }

    fn walk_block(&mut self, value: &'a mut Value) -> Result<()> {
        match value["t"].as_str().context("reading type of Block")? {
            "Para" => self.walk_inlines(&mut value["c"], "Para"),
            "Plain" => self.walk_inlines(&mut value["c"], "Plain"),
            "LineBlock" => self.walk_list_of_inlines(&mut value["c"], "LineBlock"),
            "Header" => self.walk_inlines(&mut value["c"][2], "Header"),
            "BlockQuote" => self.walk_blocks(&mut value["c"], "BlockQuote"),
            "OrderedList" => self.walk_list_of_blocks(&mut value["c"][1], "OrderedList"),
            "BulletList" => self.walk_list_of_blocks(&mut value["c"], "BulletList"),
            "Div" => self.walk_list_of_blocks(&mut value["c"][1], "Div"),
            "RawBlock" => {
                let c = &value["c"];
                let format = c[0].as_str().context("reading format of RawBlock")?;
                if format == "tex" {
                    let text = String::from(c[1].as_str().context("reading source of RawBlock")?);
                    self.add_fragment(
                        if text.trim_start().starts_with("%dontshow") {
                            FragmentType::DontShow
                        } else {
                            FragmentType::RawBlock
                        },
                        &text,
                        FragmentNodeRef::Block(value),
                    );
                }
                Ok(())
            }
            "Table" => {
                for (i, content) in value["c"]
                    .as_array_mut()
                    .context("reading contents of Table")?
                    .iter_mut()
                    .enumerate()
                // Circumvent the borrow checker ... isn't it nasty?
                {
                    match i {
                        1 => {
                            self.walk_blocks(&mut content[1], "Table.Caption")?;
                        }
                        3 => {
                            self.walk_rows(&mut content[1], "Table.TableHead")?;
                        }
                        4 => {
                            for table_body in content
                                .as_array_mut()
                                .context("reading Table.[TableBody]")?
                            {
                                for rows in table_body
                                    .as_array_mut()
                                    .context("reading content of Table.[TableBody]")?
                                    .iter_mut()
                                    .skip(2)
                                {
                                    self.walk_rows(rows, "Table.[TableBody].[Row]")?;
                                }
                            }
                        }
                        5 => {
                            self.walk_rows(&mut content[1], "Table.TableFoot")?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn walk_inline(&mut self, value: &'a mut Value) -> Result<()> {
        match value["t"].as_str().context("reading type of Inline")? {
            "Math" => {
                let c = &value["c"];
                let ty = c[0]["t"].as_str().context("reading type of Math")?;
                let text = String::from(c[1].as_str().context("reading source of Math")?);
                let ty = match ty {
                    "InlineMath" => FragmentType::InlineMath,
                    "DisplayMath" => {
                        let trimmed_text = text.trim_start();
                        if trimmed_text.starts_with("%raw") {
                            FragmentType::RawBlock
                        } else if trimmed_text.starts_with("%dontshow") {
                            FragmentType::DontShow
                        } else {
                            FragmentType::DisplayMath
                        }
                    }
                    _ => bail!("unknown math type {}", ty),
                };
                self.add_fragment(ty, &text, FragmentNodeRef::Inline(value));
                Ok(())
            }
            "Emph" => self.walk_inlines(&mut value["c"], "Emph"),
            // TODO: render them differently in latex.
            "Underline" => self.walk_inlines(&mut value["c"], "Underline"),
            "Strong" => self.walk_inlines(&mut value["c"], "Strong"),
            "Strikeout" => self.walk_inlines(&mut value["c"], "Strikeout"),
            "Link" => self.walk_inlines(&mut value["c"][1], "Link"),
            "Image" => self.walk_inlines(&mut value["c"][1], "Image"),
            _ => Ok(()),
        }
    }

    fn walk_blocks(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for block in value
            .as_array_mut()
            .with_context(|| format!("reading {}.[Block]", parent))?
            .iter_mut()
        {
            self.walk_block(block)?;
        }
        Ok(())
    }

    fn walk_list_of_blocks(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for blocks in value
            .as_array_mut()
            .with_context(|| format!("reading {}.[[Block]]", parent))?
            .iter_mut()
        {
            self.walk_blocks(blocks, parent)?;
        }
        Ok(())
    }

    fn walk_inlines(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for inline in value
            .as_array_mut()
            .with_context(|| format!("reading {}.[Inline]", parent))?
            .iter_mut()
        {
            self.walk_inline(inline)?;
        }
        Ok(())
    }

    fn walk_list_of_inlines(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for inlines in value
            .as_array_mut()
            .with_context(|| format!("reading {}.[[Inline]]", parent))?
            .iter_mut()
        {
            self.walk_inlines(inlines, parent)?;
        }
        Ok(())
    }

    fn walk_rows(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for row in value
            .as_array_mut()
            .with_context(|| format!("reading {}.[Row]", parent))?
        {
            for cell in row[1]
                .as_array_mut()
                .with_context(|| format!("reading {}.[Row].[Cell]", parent))?
            {
                self.walk_blocks(&mut cell[4], "[Cell] of Row of TableHead of Table")?;
            }
        }
        Ok(())
    }
}

fn svg_to_bboxes(node: usvg::Node) -> Vec<PathBbox> {
    let mut results = vec![];
    for node in node.descendants() {
        if !node.has_children() {
            if let Some(bbox) = node.calculate_bbox() {
                results.push(bbox);
            }
        }
    }
    results
}

/// Given a slice of bounding boxes and a y range, compute the x range that exactly covers all
/// bounding boxes which have non-empty intersection with the y range. There is a tolerance term
/// for robustness, because dvisvgm and synctex aren't always very accurate.
fn x_range_for_y_range(
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
fn refine_y_range(
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

fn split_svgs(bytes: &[u8]) -> Result<Vec<&[u8]>> {
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
