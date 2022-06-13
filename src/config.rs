use std::{env, fs};

use anyhow::{Context, Result};
use serde::Deserialize;

macro_rules! define_config {
    ($($name:ident : $type:ty = $default:expr),+$(,)?) => {
        #[derive(Clone, Debug)]
        pub struct Config {
            $(pub $name: $type,)*
        }

        #[derive(Deserialize)]
        pub struct ConfigUpdate {
            $(#[serde(default)] $name: Option<$type>,)*
        }

        impl Config {
            fn update(&mut self, update: ConfigUpdate) {
                $(if let Some($name) = update.$name { self.$name = $name; })*
            }
        }

        impl Default for Config {
            fn default() -> Self {
                Self {
                    $($name: $default.into(),)*
                }
            }
        }
    };
}

define_config! {
    preamble: String = r"
\documentclass[12pt, fleqn]{article}
\usepackage[top=0cm, bottom=0cm, left=0cm, right=0cm, paperheight=16000pt]{geometry}
\usepackage{amsmath, amssymb, amsthm, bm}
\setlength{\parindent}{0pt}
\begin{document}",
    postamble: String = r"\end{document}",
    // Path to the latex executable.
    latex: String = "pdflatex",
    // Path to dvisvgm.
    dvisvgm: String = "dvisvgm",
    // Defines the error tolerance for x_range_for_y_range procedure in main.rs.
    // See that function for what this exactly means.
    y_range_tol: f64 = 1.0,
    // A blank margin to rendered elements. The unit is pt.
    // Tune it if you think the inline fragments are too close to the
    // surrounding text.
    x_range_margin: f64 = 1.0,
    // Adjustment to inline rendering of fragments. The unit is pt.
    // A positive value makes inline fragments higher.
    baseline_rise: f64 = 0.0,
    // The tag that inserts the LZMA decompressor.
    lzma_script: String = 
        r#"<script src="https://cdn.jsdelivr.net/npm/lzma@2.3.2/src/lzma-d-min.js"></script>"#,
}

impl Config {
    pub fn load() -> Self {
        let mut result = Self::default();
        let _ = result.load_from_current_exe();
        let _ = result.load_from_working_dir();
        result
    }

    fn load_from_current_exe(&mut self) -> Result<()> {
        let exe_path = env::current_exe()?;
        let config_path = exe_path
            .parent()
            .context("exe path has no parent")?
            .join("jlconfig.toml");
        let content = fs::read_to_string(config_path)?;
        self.update(toml::from_str(&content)?);
        eprintln!("Loaded configs from exe directory");
        Ok(())
    }

    fn load_from_working_dir(&mut self) -> Result<()> {
        let content = fs::read_to_string("jlconfig.toml")?;
        self.update(toml::from_str(&content)?);
        eprintln!("Loaded configs from working directory");
        Ok(())
    }
}
