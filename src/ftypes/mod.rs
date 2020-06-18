use tree_sitter as ts;

use crate::{Error, Result};

mod ftype_toml;
mod ftype_txt_en;
mod ftype_txt_plain;

pub use ftype_txt_en::TextEn;
pub use ftype_txt_plain::PlainText;

pub fn new_parser(content: &str, lang: ts::Language) -> Result<(ts::Parser, ts::Tree)> {
    let mut parser = {
        let mut parser = ts::Parser::new();
        err_at!(FailParse, parser.set_language(lang))?;
        parser
    };
    let tree = parser.parse(content, None).unwrap();
    Ok((parser, tree))
}