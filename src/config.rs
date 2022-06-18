use std::{env, path::Path};

use anyhow::{format_err, Context, Result};
use indoc::indoc;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub preamble: String,
    pub postamble: String,
    /// Path to the latex executable.
    pub latex: String,
    /// Path to the dvisvgm executable.
    pub dvisvgm: String,
    /// Defines the error tolerance for [`crate::x_range_for_y_range`] and
    /// [`crate::refine_y_range`].
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
    /// The tag that inserts the LZMA decompressor.
    ///
    /// But really it can also include anything you want to insert along at the end of the HTML.
    pub lzma_script: String,
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
    /// Template for inline math.
    pub inline_math: String,
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
    pub fn load() -> Result<Self> {
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
            .set_default("y_range_tol", 0.0)?
            .set_default("x_range_margin", 1.0)?
            .set_default("y_range_margin", 1.0)?
            .set_default("baseline_rise", 0.0)?
            .set_default("lzma_script", 
            r#"<script src="https://cdn.jsdelivr.net/npm/lzma@2.3.2/src/lzma-d-min.js"></script>"#)?
            .set_default("output_folder", Option::<String>::None)?
            // Default templates...
            .set_default("template.placeholder", "{{fragment}}")?
            .set_default("template.inline_math", r"\({{fragment}}\)")?
            .set_default("template.display_math", indoc! {r"\[
                    {{fragment}}
                \]"})?
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

        c.build()?
            .try_deserialize()
            .map_err(|e| format_err!("cannot load config: {}", e))
    }
}
