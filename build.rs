use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write;
use std::fs;
use std::path::Path;

fn main() {
    const CATALOG: &str = "assets/mice/catalog.json";
    println!("cargo:rerun-if-changed={CATALOG}");
    println!("cargo:rerun-if-changed=assets/mice/renders");
    let entries: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(CATALOG).expect("read illustration catalog"))
            .expect("parse illustration catalog");
    let mut registered = BTreeSet::new();
    let mut source =
        String::from("fn by_pid(pid: u16) -> Option<&'static Illustration> {\n    match pid {\n");

    for (slug, entry) in entries {
        let asset = format!("assets/mice/renders/{slug}.webp");
        let pids = entry["pids"].as_array().expect("product ID array");
        assert!(!pids.is_empty(), "missing product IDs for {slug}");
        assert!(Path::new(&asset).is_file(), "missing {asset}");
        let pattern = pids
            .iter()
            .map(|pid| {
                let hex = pid.as_str().expect("hex product ID");
                let number = u16::from_str_radix(hex, 16).expect("valid product ID");
                assert!(registered.insert(number), "duplicate product ID {hex}");
                format!("{number:#06x}")
            })
            .collect::<Vec<_>>()
            .join(" | ");
        writeln!(
            source,
            "        {pattern} => Some(&Illustration {{ id: {slug:?}, bytes: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/\", {asset:?})) }}),",
        )
        .expect("write illustration entry");
    }
    source.push_str("        _ => None,\n    }\n}\n");
    writeln!(
        source,
        "#[cfg(test)]\nconst PRODUCT_IDS: &[u16] = &{:?};",
        registered.into_iter().collect::<Vec<_>>(),
    )
    .expect("write product IDs");
    let out = env::var_os("OUT_DIR").expect("Cargo output directory");
    fs::write(Path::new(&out).join("mouse_illustrations.rs"), source)
        .expect("write compiled illustration catalog");
}
