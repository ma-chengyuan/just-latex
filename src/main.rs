use anyhow::{bail, Context as AnyhowContext, Result};
use bytesize::ByteSize;
use config::Config;
use indoc::formatdoc;
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{stdin, stdout, Cursor, Read, Write},
    ops::Range,
    process::Command,
    str::FromStr,
    time::Instant,
    vec,
};
use tempfile::TempDir;
use usvg::{NodeExt, PathBbox};
use xz2::{read::XzEncoder, stream::LzmaOptions};

use crate::synctex::Scanner;

mod config;
mod synctex;

fn main() -> Result<()> {
    let config = Config::load();
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
            match item.ty {
                FragmentType::InlineMath => output.push('$'),
                FragmentType::DisplayMath => {
                    output.push_str("\\[\n");
                    current_line += 1;
                }
                FragmentType::RawBlock | FragmentType::DontShow => {}
            }
            let start_line = current_line;
            output.push_str(&item.src);
            current_line += item.src.lines().count();
            match item.ty {
                FragmentType::InlineMath => output.push('$'),
                FragmentType::DisplayMath => {
                    output.push_str("\n\\]");
                    current_line += 1;
                }
                FragmentType::RawBlock | FragmentType::DontShow => {}
            }
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

        // dvisvgm does very spurious scaling to the output svg even when no magnification arguments
        // are passed. Besides the viewboxes are very weird.
        // x_svg = x_tex * 0.996264;
        // y_svg = y_tex * 0.996264 - page_height;
        const TEX2SVG_SCALING: f64 = 72.0 / 72.27;

        let (source_str, lines) = self.generate_latex_with_line_mappings();
        let working_dir = TempDir::new()?;
        let working_path = working_dir.path().canonicalize()?;
        // let working_path = std::path::Path::new("./output").canonicalize()?;
        let source_path = working_path.join("source.tex");
        {
            let mut source = File::create(&source_path)?;
            source.write_all(source_str.as_bytes())?;
        }
        let pdf_path = working_path.join("source.pdf");
        let latex_command = Command::new("pdflatex")
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
        let dvisvgm_command = Command::new("dvisvgm")
            .args(["-s", "-R", "-P", pdf_path.to_str().unwrap()])
            .current_dir(&working_path)
            .output()?;
        if !dvisvgm_command.status.success() {
            bail!("fail to run dvisvgm");
        }
        let options = usvg::Options::default();

        let svg = usvg::Tree::from_data(&dvisvgm_command.stdout, &options.to_ref())?;
        let svg_data = dvisvgm_command.stdout;
        let mut bboxes = vec![];
        // Compute bounding boxes of leaf SVG elements. This will be useful later.
        svg_to_bboxes(svg.root(), &mut bboxes);

        let viewbox_top = svg.svg_node().view_box.rect.top();

        let scanner = Scanner::new(pdf_path, &working_path);
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
            // eprintln!("{:#?} {}", item.ty, item.src);
            let mut y_range = (f64::MAX, f64::MIN);
            let mut x_range = (f64::MAX, f64::MIN);
            // We need to know the baseline in order to align the svgs to the baseline of the
            // surrounding text.
            let mut baseline = (0.0, 0.0);
            for line in line_range {
                for tb in scanner.query(line) {
                    if tb.width > baseline.1 {
                        baseline = (
                            tb.v * TEX2SVG_SCALING + viewbox_top,
                            tb.width * TEX2SVG_SCALING,
                        );
                    }
                    y_range = (
                        y_range
                            .0
                            .min((tb.v - tb.height) * TEX2SVG_SCALING + viewbox_top),
                        y_range
                            .1
                            .max((tb.v + tb.depth) * TEX2SVG_SCALING + viewbox_top),
                    );
                    x_range = (
                        x_range.0.min(tb.h * TEX2SVG_SCALING),
                        x_range.1.min(tb.h + tb.width * TEX2SVG_SCALING),
                    )
                }
            }
            if y_range == (f64::MAX, f64::MIN) {
                // which means we haven't encounter a single box...
                bail!("no box found for equation {}", item.src);
            }
            // eprintln!("{:#?}", y_range);
            // The x bound given by synctex is not always accurate. Most of the time adding an
            // \hfill will create a box that horizontally span the entire page. However we only care
            // about visible parts. Here we look into the svg for more precise x bounds.
            let x_range = x_range_for_y_range(
                &bboxes,
                y_range.0,
                y_range.1,
                self.config.y_range_tol,
                self.config.x_range_margin,
            )
            .unwrap_or(x_range);

            let depth = match item.ty {
                FragmentType::InlineMath => y_range.1 - baseline.0,
                FragmentType::DisplayMath | FragmentType::RawBlock => 0.0,
                FragmentType::DontShow => unreachable!(),
            };
            let svg_elem = formatdoc!(
                r##"<img src="#svgView(viewBox({:.2},{:.2},{width:.2},{height:.2}))" 
                         class="svg-math" 
                         style="width:{width:.2}pt;height:{height:.2}pt;
                         top:{depth:.2}pt;position:relative;">"##,
                x_range.0,
                y_range.0,
                width = x_range.1 - x_range.0,
                height = y_range.1 - y_range.0,
                depth = depth - self.config.baseline_rise,
            );
            let svg_elem = match item.ty {
                FragmentType::InlineMath => svg_elem,
                FragmentType::DisplayMath | FragmentType::RawBlock => {
                    format!("<p>{}</p>", svg_elem)
                }
                FragmentType::DontShow => unreachable!(),
            };
            for node in item.refs.iter_mut() {
                match node {
                    FragmentNodeRef::Inline(node) => {
                        **node = json!({"t": "RawInline", "c": ["html", &svg_elem]});
                    }
                    FragmentNodeRef::Block(node) => {
                        **node = json!({"t": "RawBlock", "c": ["html", &svg_elem]});
                    }
                }
            }
        }
        let start = Instant::now();
        let original_size = svg_data.len();
        let lzma_options = LzmaOptions::new_preset(9)?;
        let mut svg_compressor = XzEncoder::new_stream(
            Cursor::new(svg_data),
            xz2::stream::Stream::new_lzma_encoder(&lzma_options)?,
        );
        // let mut svg_compressor = brotli::CompressorReader::new(Cursor::new(svg_data), 4096, 9, 20);
        let mut svg_compressed = vec![];
        svg_compressor.read_to_end(&mut svg_compressed)?;
        let svg_encoded = base64::encode(svg_compressed);
        eprintln!(
            "SVG compressed from {} down to {} (base64 encoded) in {}s",
            ByteSize::b(original_size as u64),
            ByteSize::b(svg_encoded.len() as u64),
            start.elapsed().as_secs_f64()
        );
        let final_code = formatdoc!(
            r##"
            <script src="https://cdn.jsdelivr.net/npm/lzma@2.3.2/src/lzma_worker-min.js"></script>
            <script>
                LZMA.decompress(Uint8Array.from(atob("{}"), function(c) {{ return c.charCodeAt(0); }}), 
                    function(result, error) {{
                        var svgUrl = URL.createObjectURL(new Blob([result], {{type: "image/svg+xml"}}));
                        var imgs = document.getElementsByClassName("svg-math");
                        console.log(imgs.length);
                        for (var i = 0; i < imgs.length; i++) {{
                            var hashPos = imgs[i].src.indexOf("#");
                            if (hashPos != -1)
                                imgs[i].src = svgUrl + imgs[i].src.substring(hashPos);
                        }}
                    }}, 
                    function(p) {{}}
                );
            </script>
        "##,
            svg_encoded
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
            .with_context(|| format!("reading [Block] of {}", parent))?
            .iter_mut()
        {
            self.walk_block(block)?;
        }
        Ok(())
    }

    fn walk_list_of_blocks(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for blocks in value
            .as_array_mut()
            .with_context(|| format!("reading [[Block]] of {}", parent))?
            .iter_mut()
        {
            self.walk_blocks(blocks, parent)?;
        }
        Ok(())
    }

    fn walk_inlines(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for inline in value
            .as_array_mut()
            .with_context(|| format!("reading [Inline] of {}", parent))?
            .iter_mut()
        {
            self.walk_inline(inline)?;
        }
        Ok(())
    }

    fn walk_list_of_inlines(&mut self, value: &'a mut Value, parent: &str) -> Result<()> {
        for inlines in value
            .as_array_mut()
            .with_context(|| format!("reading [[Inline]] of {}", parent))?
            .iter_mut()
        {
            self.walk_inlines(inlines, parent)?;
        }
        Ok(())
    }
}

fn svg_to_bboxes(node: usvg::Node, results: &mut Vec<PathBbox>) {
    if node.has_children() {
        for ch in node.children() {
            svg_to_bboxes(ch, results);
        }
    } else if let Some(bbox) = node.calculate_bbox() {
        results.push(bbox);
    }
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
