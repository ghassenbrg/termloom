//! Public-API integration test for the required local VSIX lifecycle.

use std::fs::File;
use std::io::Write;

use tempfile::tempdir;
use termloom::domain::extensions::CompatibilityClass;
use termloom::services::extensions::{install_into, list_in, load_snippets, remove_from};
use zip::write::SimpleFileOptions;

#[test]
fn installs_loads_lists_and_removes_declarative_assets() {
    let temp = tempdir().unwrap();
    let package = temp.path().join("portable.vsix");
    let root = temp.path().join("extensions");
    let file = File::create(&package).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    archive
        .start_file("extension/package.json", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(
            br#"{
              "name": "portable",
              "publisher": "termloom-test",
              "version": "1.2.3",
              "contributes": {
                "languages": [{"id":"loom","extensions":[".loom"]}],
                "snippets": [{"language":"loom","path":"snippets/loom.json"}],
                "themes": [{"label":"Loom Dark","uiTheme":"vs-dark","path":"themes/dark.json"}]
              }
            }"#,
        )
        .unwrap();
    archive
        .start_file("extension/snippets/loom.json", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(br#"{"Loom block":{"prefix":"loom","body":["fn ${1:name}() {","  $0","}"]}}"#)
        .unwrap();
    archive
        .start_file("extension/themes/dark.json", SimpleFileOptions::default())
        .unwrap();
    archive
        .write_all(br##"{"colors":{"editor.background":"#101820"}}"##)
        .unwrap();
    archive.finish().unwrap();

    let installed = install_into(&package, &root).unwrap();
    assert_eq!(installed.id, "termloom-test.portable");
    assert_eq!(installed.class, CompatibilityClass::Full);

    let listed = list_in(&root).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, installed.id);
    let snippets = load_snippets(&listed);
    let loom = &snippets["loom"];
    assert_eq!(loom.len(), 1);
    assert_eq!(loom[0].label, "loom");
    assert_eq!(loom[0].insert_text, "fn name() {\n  \n}");

    assert!(remove_from(&installed.id, &root).unwrap());
    assert!(list_in(&root).unwrap().is_empty());
}
