pub mod tsconfig;

use moon_lang::Language;
pub use tsconfig::TsConfigJson;

pub const TYPESCRIPT: Language = Language {
    binary: "tsc",
    default_version: "4.8.4",
    file_exts: &["ts", "tsx", "cts", "mts", "d.ts", "d.cts", "d.mts"],
    vendor_bins_dir: None,
    vendor_dir: Some("node_modules/@types"),
};
