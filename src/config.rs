use std::{env, path::Path};

use anyhow::{bail, format_err, Context, Result};
use config::{builder::DefaultState, ConfigBuilder};
use indoc::indoc;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub preamble: String,
    pub postamble: String,
    /// Path to the latex executable.
    pub latex: String,
    /// Path to the dvisvgm executable.
    pub dvisvgm: String,
    /// Operating mode, either "pdf" or "dvi" or "xdv".
    // Should have really made this an enum. But writing manual impls for Deserialize does not seem
    // to worth the effort.
    pub mode: String,
    /// Defines the error tolerance for [`crate::x_range_for_y_range`] and
    /// [`crate::refine_y_range`].dvi_
    pub y_range_tol: f64,
    /// A blank horizontal margin to rendered inline fragments. The unit is pt.
    ///
    /// Tune it if you think the inline fragments are too close to the
    /// surrounding text.
    pub x_range_margin: f64,
    /// A blank vertical margin to rendered block fragments. The unit is pt.
    pub y_range_margin: f64,
    /// Adjustment to inline rendering of fragments. The unit is pt.
    ///
    /// A positive value makes inline fragments higher.
    pub baseline_rise: f64,
    /// Path to lzma-d-min.js.
    pub lzma_js_path: String,
    /// Extra attributes to the decompressor <script> tag.
    ///
    /// For instance, in some pjax implementations a script needs to have data-pjax as
    /// as attribute for it to be executed when the page loads.
    pub script_extra_attributes: String,

    /// Extra styles to be inserted to inline rendered <imgs>.
    ///
    /// Or, alternatively, all inline <imgs> are accessible via `.jl-inline`, so you can also
    /// include extra styling in some separate CSS.
    pub extra_style_inline: String,
    /// Extra styles to be inserted to display rendered <imgs>.
    ///
    /// Or, alternatively, all display <imgs> are accessible via `.jl-display`, so you can also
    /// include extra styling in some separate CSS.
    pub extra_style_display: String,

    /// Configuration related to templating of fragments.
    pub template: TemplateConfig,
    /// Configuration for the SVG optimizer.
    pub optimizer: OptimizerConfig,
    /// Output folder for intermediate files. Useful in case of LaTeX compilation errors.
    /// If none, the program dumps everything in a temp folder.
    pub output_folder: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TemplateConfig {
    /// The placeholder that will be replaced by the fragment content for all templates below.
    pub placeholder: String,
    /// Template for inline math
    pub inline_math: String,
    pub inline_math_inner: String,
    // Style elements
    pub strong: String,
    pub emph: String,
    pub quote: String,
    pub header: Vec<String>,
    /// Template for display math.
    pub display_math: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct OptimizerConfig {
    /// Is the optimizer enabled?
    pub enabled: bool,
    /// The precision bound for path similarity checks.
    pub eps: f64,
}

impl Config {
    /// Loads configuration from config files, as well as document.
    pub fn load(tree: &Value) -> Result<Self> {
        let placeholder = "{{fragment}}";
        let mut c = config::Config::builder()
            .set_default(
                "preamble",
                indoc! {r"
                    \documentclass[12pt, fleqn]{article}
                    \usepackage[top=0cm, bottom=0cm, left=0cm, right=0cm, paperheight=16000pt]{geometry}
                    \usepackage{amsmath, amssymb, amsthm, bm}
                    \setlength{\parindent}{0pt}
                    \begin{document}"
                },
            )?
            .set_default("postamble", r"\end{document}")?
            .set_default("latex", "pdflatex")?
            .set_default("dvisvgm", "dvisvgm")?
            .set_default("mode", "pdf")?
            .set_default("y_range_tol", 0.0)?
            .set_default("x_range_margin", 1.0)?
            .set_default("y_range_margin", 1.0)?
            .set_default("baseline_rise", 0.0)?
            .set_default("lzma_js_path", "https://cdn.jsdelivr.net/npm/lzma@2/src/lzma-d-min.js")?
            .set_default("script_extra_attributes", "")?
            .set_default("extra_style_inline", "")?
            .set_default("extra_style_display", "")?
            .set_default("output_folder", Option::<String>::None)?
            // Default templates...
            .set_default("template.placeholder", placeholder)?
            .set_default("template.inline_math", format!(r"\({}\)", placeholder))?
            .set_default("template.inline_math_inner", placeholder)?
            .set_default("template.inline_quote", placeholder)?
            .set_default("template.emph", placeholder)?
            .set_default("template.strong", placeholder)?
            .set_default("template.quote", placeholder)?
            .set_default("template.header", 
                ["24", "18", "14.04", "12", "9.96", "8.04"]
                .map(|pt| format!(r"\text{{\fontsize{{{}pt}}{{0}}\selectfont${}$}}", pt, placeholder))
                .to_vec()
            )?
            .set_default("template.display_math", format!("\\[\n    {}\n\\]", placeholder))?
            .set_default("optimizer.enabled", false)?
            .set_default("optimizer.eps", 0.001)?;

        let exe_config = env::current_exe()?.join("jlconfig.toml");
        if exe_config.exists() {
            c = c.add_source(config::File::new(
                exe_config
                    .to_str()
                    .context("cannot convert path to string")?,
                config::FileFormat::Toml,
            ));
        }

        if Path::new("jlconfig.toml").exists() {
            c = c.add_source(config::File::new("jlconfig.toml", config::FileFormat::Toml));
        }

        for (key, value) in tree["meta"]
            .as_object()
            .context("reading document metadata")?
        {
            if key == "jlconfig" {
                if value["t"] != "MetaMap" {
                    bail!("in Front Matter configuration, jlconfig must be a map!");
                }
                for (sub_key, value) in value["c"].as_object().context("reading map of MetaMap")? {
                    c = walk_meta(c, value, sub_key)?;
                }
            } else if let Some(key) = key.strip_prefix("jlconfig.") {
                c = walk_meta(c, value, key)?;
            }
        }

        c.build()?
            .try_deserialize()
            .map_err(|e| format_err!("cannot load config: {}", e))
    }

    pub fn sanity_check(&self) -> Result<()> {
        if self.mode != "pdf" && self.mode != "dvi" && self.mode != "xdv" {
            bail!("unknown mode: must be one of 'pdf', 'dvi', or 'xdv'");
        }
        if self.mode != "pdf" && self.optimizer.enabled {
            bail!("DVI/XDV mode is incompatible with JustLaTeX's SVG optimizer");
        }
        Ok(())
    }
}

fn walk_meta(
    mut cb: ConfigBuilder<DefaultState>,
    value: &Value,
    key: &str,
) -> Result<ConfigBuilder<DefaultState>> {
    match value["t"].as_str().context("reading type of Meta")? {
        "MetaInlines" => {
            let mut content = String::new();
            meta_inlines_to_string(&value["c"], &mut content)?;
            Ok(cb.set_override(key, content)?)
        }
        "MetaBlocks" => {
            let mut content = String::new();
            meta_blocks_to_string(&value["c"], &mut content)?;
            Ok(cb.set_override(key, content)?)
        }
        "MetaMap" => {
            for (sub_key, value) in value["c"].as_object().context("reading map of MetaMap")? {
                cb = walk_meta(cb, value, &format!("{}.{}", key, sub_key))?;
            }
            Ok(cb)
        }
        "MetaList" => {
            for (i, value) in value["c"]
                .as_array()
                .context("reading array of MetaList")?
                .iter()
                .enumerate()
            {
                cb = walk_meta(cb, value, &format!("{}[{}]", key, i))?;
            }
            Ok(cb)
        }
        "MetaString" => Ok(cb.set_override(
            key,
            value["c"].as_str().context("reading value of MetaString")?,
        )?),
        "MetaBool" => Ok(cb.set_override(
            key,
            value["c"].as_bool().context("reading value of MetaBool")?,
        )?),
        ty => bail!("unsupported Meta type: {}", ty),
    }
}

fn meta_inlines_to_string(value: &Value, content: &mut String) -> Result<()> {
    for value in value.as_array().context("reading Meta.[Inline]")? {
        match value["t"].as_str().context("reading type of Meta.Inline")? {
            "Str" => content.push_str(
                value["c"]
                    .as_str()
                    .context("reading content of Meta.Inline.Str")?,
            ),
            "RawInline" | "Code" | "Math" => content.push_str(
                value["c"][1]
                    .as_str()
                    .context("reading content of Meta.Inline.RawInline/Code/Math")?,
            ),
            "Emph" | "Strong" | "Underline" => meta_inlines_to_string(&value["c"], content)?,
            "Space" => content.push(' '),
            ty => bail!("unsupported Inline type in Meta: {}", ty),
        }
    }
    Ok(())
}

fn meta_blocks_to_string(value: &Value, content: &mut String) -> Result<()> {
    for value in value.as_array().context("reading Meta.[Block]")? {
        match value["t"].as_str().context("reading type of Meta.Block")? {
            "Plain" | "Para" => meta_inlines_to_string(&value["c"], content)?,
            "RawBlock" | "CodeBlock" => content.push_str(
                value["c"][1]
                    .as_str()
                    .context("reading content of Meta.Block.RawBlock/CodeBlock")?,
            ),
            ty => bail!("unsupported Inline type in Meta: {}", ty),
        }
        content.push('\n');
    }
    Ok(())
}
