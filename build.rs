use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=synctex_wrapper.h");
    let bindings = bindgen::Builder::default()
        .header("synctex/synctex_parser.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("couldn't write bindings!");
    cc::Build::new()
        .file("synctex/synctex_parser.c")
        .file("synctex/synctex_parser_utils.c")
        .include("synctex")
        .warnings(false)
        .compile("synctex");    
    println!("cargo:rustc-link-lib=static=z");
}
