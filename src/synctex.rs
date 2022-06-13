#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use std::{ffi::CString, os::raw::c_int, path::Path};

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

pub struct Scanner(synctex_scanner_p);

impl Scanner {
    pub fn new(output: impl AsRef<Path>, build_dir: impl AsRef<Path>) -> Self {
        let output = CString::new(output.as_ref().to_string_lossy().as_bytes()).unwrap();
        let build_dir = CString::new(build_dir.as_ref().to_string_lossy().as_bytes()).unwrap();
        Self(unsafe {
            synctex_scanner_new_with_output_file(output.as_ptr(), build_dir.as_ptr(), 1)
        })
    }

    pub fn query(&self, line: usize) -> Vec<TeXBox> {
        fn f(x: i32) -> f64 {
            x as f64 / 65536.0
        }

        unsafe {
            let name = synctex_scanner_get_name(self.0, 1);
            let result = synctex_display_query(self.0, name, line as c_int, 0, -1);
            let mut ret = vec![];
            if result > 0 {
                let mut node = synctex_scanner_next_result(self.0);
                while !node.is_null() {
                    let h = f(synctex_node_box_h(node));
                    let v = f(synctex_node_box_v(node));
                    let height = f(synctex_node_box_height(node));
                    let width = f(synctex_node_box_width(node));
                    let depth = f(synctex_node_box_depth(node));
                    /* 
                    There seems to be some precision issues with these functions???
                    let h = synctex_node_visible_h(node) as f64;
                    let v = synctex_node_visible_v(node) as f64;
                    let height = synctex_node_visible_height(node) as f64;
                    let width = synctex_node_visible_width(node) as f64;
                    let depth = synctex_node_visible_depth(node) as f64;
                    */

                    ret.push(TeXBox { h, v, height, width, depth });

                    /*
                    eprintln!("h: {}", f(synctex_node_box_h(node)));
                    eprintln!("v: {}", f(synctex_node_box_v(node)));
                    eprintln!("height: {}", f(synctex_node_box_height(node)));
                    eprintln!("width: {}", f(synctex_node_box_width(node)));
                    eprintln!("depth: {}", f(synctex_node_box_depth(node)));
                    eprintln!("visible h: {}", synctex_node_box_visible_h(node));
                    eprintln!("visible v: {}", synctex_node_box_visible_v(node));
                    eprintln!("visible width: {}", synctex_node_box_visible_width(node));
                    eprintln!("visible depth: {}", synctex_node_box_visible_depth(node));
                    eprintln!("visible height: {}", synctex_node_box_visible_height(node));
                    */
                    node = synctex_scanner_next_result(self.0);
                }
            }
            ret
        }
    }

    pub fn dump(&self) {
        unsafe {
            synctex_scanner_display(self.0);
        }
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        unsafe {
            synctex_scanner_free(self.0);
        }
    }
}

#[derive(Clone, Debug)]
pub struct TeXBox {
    pub h: f64,
    pub v: f64,
    pub height: f64, 
    pub width: f64,
    pub depth: f64
}