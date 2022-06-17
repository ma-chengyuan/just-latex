#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

use std::{ffi::CString, os::raw::c_int, path::Path, hash::Hash};

use ordered_float::OrderedFloat;

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
        fn texpt_to_f64(x: i32) -> OrderedFloat<f64> {
            (x as f64 / 65536.0).into()
        }

        unsafe {
            let name = synctex_scanner_get_name(self.0, 1);
            let result = synctex_display_query(self.0, name, line as c_int, 0, -1);
            let mut ret = vec![];
            if result > 0 {
                let mut node = synctex_scanner_next_result(self.0);
                while !node.is_null() {
                    ret.push(TeXBox {
                        h: texpt_to_f64(synctex_node_box_h(node)),
                        v: texpt_to_f64(synctex_node_box_v(node)),
                        height: texpt_to_f64(synctex_node_box_height(node)),
                        width: texpt_to_f64(synctex_node_box_width(node)),
                        depth: texpt_to_f64(synctex_node_box_depth(node)),
                        page: synctex_node_page(node) as u32,
                        // ty: String::from(CStr::from_ptr(synctex_node_isa(node)).to_str().unwrap())
                    });
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

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TeXBox {
    pub h: OrderedFloat<f64>,
    pub v: OrderedFloat<f64>,
    pub height: OrderedFloat<f64>,
    pub width: OrderedFloat<f64>,
    pub depth: OrderedFloat<f64>,
    pub page: u32,
    // pub ty: String,
}