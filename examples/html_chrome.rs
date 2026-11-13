//! Spike: the Canopy tree strip drawn in HTML over the page, on tao + wry.
//!
//! cargo run --example html_chrome --features html-spike -- [DATA_DIR] [ADDRESS]
//!
//! DATA_DIR defaults to `.local-data/html-spike`, so the spike never touches your real forest.

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let data_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".local-data/html-spike"));
    let start = args.next();
    if let Err(error) = canopy::spike::run(&data_dir, start.as_deref()) {
        eprintln!("Canopy spike: {error:#}");
        std::process::exit(1);
    }
}
