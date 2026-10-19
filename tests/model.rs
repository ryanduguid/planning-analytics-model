//! Loading the manifest, dimensions, cubes and processes of a model tree, and
//! the shipped model's own structure and rules discipline.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use pacioliscube::data::read_csv;
use pacioliscube::decimal::Decimal;
use pacioliscube::error::Error;
use pacioliscube::model::{
    Model, dimension_id, load_cube, load_dimension, load_model, load_process,
};
use pacioliscube::py;
use pacioliscube::pyjson::{self, Json};
use pacioliscube::pypath::PyPath;
use pacioliscube::validate::validate_model;

fn pp(path: &Path) -> PyPath {
    PyPath::new(&s(path))
}

fn load(root: &Path) -> Result<Model, Error> {
    load_model(&pp(root))
}

fn model_error(result: Result<impl std::fmt::Debug, Error>) -> String {
    match result {
        Err(Error::Model(m)) => m,
        other => panic!("expected a model error, got {other:?}"),
    }
}

fn codes(model: &Model) -> Vec<&'static str> {
    validate_model(model)
        .unwrap()
        .iter()
        .map(|f| f.code)
        .collect()
}

/// `json.dumps` of an edited JSON file, as the Python tests wrote them.
fn edit_json(path: &Path, change: impl FnOnce(&mut Json)) {
    let mut value = pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap();
    change(&mut value);
    std::fs::write(path, to_json(&value)).unwrap();
}

fn to_json(value: &Json) -> String {
    match value {
        Json::Null => "null".into(),
        Json::Bool(b) => b.to_string(),
        Json::Int(i) => i.to_string(),
        Json::Float(f) => py::float_repr(*f),
        Json::Str(s) => {
            let mut out = String::new();
            pyjson::dump_str(s, &mut out);
            out
        }
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(to_json).collect::<Vec<_>>().join(", ")
        ),
        Json::Object(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| {
                    let mut key = String::new();
                    pyjson::dump_str(k, &mut key);
                    format!("{key}: {}", to_json(v))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn set_key(value: &mut Json, key: &str, new: Json) {
    let Json::Object(pairs) = value else { panic!() };
    match pairs.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = new,
        None => pairs.push((key.into(), new)),
    }
}

fn remove_key(value: &mut Json, key: &str) {
    let Json::Object(pairs) = value else { panic!() };
    pairs.retain(|(k, _)| k != key);
}

fn field<'a>(value: &'a mut Json, key: &str) -> &'a mut Json {
    let Json::Object(pairs) = value else { panic!() };
    &mut pairs.iter_mut().find(|(k, _)| k == key).unwrap().1
}

fn push(value: &mut Json, item: Json) {
    let Json::Array(items) = value else { panic!() };
    items.push(item);
}

fn j(text: &str) -> Json {
    pyjson::loads(text).unwrap()
}

fn write_manifest(root: &Path, objects: &str) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("tm1project.json"),
        format!(r#"{{"Version": 1.0, "Name": "x", "Objects": {objects}}}"#),
    )
    .unwrap();
}

fn write_cube(root: &Path, name: &str, fields: &str) -> PathBuf {
    let path = root.join(format!("{name}.json"));
    std::fs::write(&path, format!(r#"{{"Name": "{name}"{fields}}}"#)).unwrap();
    path
}

fn colour_hierarchy(root: &Path) -> PathBuf {
    root.join("dimensions")
        .join("Colour.hierarchies")
        .join("Colour.json")
}

fn validate_cli(root: &Path) -> (i32, String, String) {
    run(&["validate", &s(root)])
}

// ---------------------------------------------------------------- manifest

#[test]
fn model_loads_every_object_the_manifest_lists() {
    let model = load(&mini()).unwrap();
    let dims: Vec<&str> = model.dimensions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(dims, ["Colour", "Measure"]);
    assert_eq!(model.cube_names(), ["Sales", "Cost"]);
    let processes: Vec<&str> = model.processes.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(processes, ["Load"]);
    assert_eq!(
        model.cube("Sales").unwrap().dimensions,
        ["Colour", "Measure"]
    );
}

#[test]
fn a_cube_loads_on_its_own_in_both_forms() {
    let cube = load_cube(&pp(&mini().join("cubes").join("Sales.json")), None).unwrap();
    assert_eq!(cube.dimensions, ["Colour", "Measure"]);
    let tmp = tempdir();
    let root = write_model(tmp.path(), &Spec::default());
    let cube = load_cube(&pp(&root.join("cubes").join("Sales.json")), None).unwrap();
    assert_eq!(cube.dimensions, ["Colour", "Measure"]);
}

#[test]
fn dimension_references_follow_the_odata_form() {
    let tmp = tempdir();
    let path = write_cube(
        tmp.path(),
        "Ghost",
        r#", "Dimensions": [{"@id": "Dimensions('O''Brien')"}]"#,
    );
    assert_eq!(load_cube(&pp(&path), None).unwrap().dimensions, ["O'Brien"]);
    for entry in [
        r#""Colour""#,
        "{}",
        r#"{"@id": "Hierarchies('Colour')"}"#,
        r#"{"@id": "Dimensions('')"}"#,
        r#"{"@id": "Dimensions('Colour')/Hierarchies('Colour')"}"#,
    ] {
        let path = write_cube(
            tmp.path(),
            "Ghost",
            &format!(r#", "Dimensions": [{entry}]"#),
        );
        assert!(
            model_error(load_cube(&pp(&path), None)).contains("dimension reference"),
            "{entry}"
        );
    }
}

#[test]
fn a_cube_giving_its_dimensions_both_ways_is_refused() {
    for dimensions in [r#"[{"@id": "Dimensions('Colour')"}]"#, "[]", "null"] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        let path = write_cube(
            &root.join("cubes"),
            "Ghost",
            &format!(
                r#", "Dimensions": {dimensions}, "Dimensions@Code.links": ["../dimensions/Colour.json"]"#
            ),
        );
        assert!(
            model_error(load_cube(&pp(&path), None))
                .contains("both as Dimensions and as Dimensions@Code.links")
        );
    }
}

#[test]
fn a_cube_naming_no_dimensions_or_not_a_list_is_refused() {
    let tmp = tempdir();
    for fields in [
        "",
        r#", "Dimensions": []"#,
        r#", "Dimensions": null"#,
        r#", "Dimensions@Code.links": []"#,
    ] {
        let path = write_cube(tmp.path(), "Ghost", fields);
        assert!(
            model_error(load_cube(&pp(&path), None)).contains("names no dimensions"),
            "{fields}"
        );
    }
    for form in ["Dimensions", "Dimensions@Code.links"] {
        let path = write_cube(
            tmp.path(),
            "Ghost",
            &format!(r#", "{form}": {{"@id": "Dimensions('Colour')"}}"#),
        );
        assert!(
            model_error(load_cube(&pp(&path), None)).contains(&format!("{form} must be a list"))
        );
    }
}

#[test]
fn a_reference_to_a_dimension_the_model_lacks_fails_validation() {
    let tmp = tempdir();
    let root = write_model(tmp.path(), &Spec::default());
    write_cube(
        &root.join("cubes"),
        "Sales",
        r#", "Dimensions": [{"@id": "Dimensions('Colour')"}, {"@id": "Dimensions('Ghost')"}]"#,
    );
    assert!(codes(&load(&root).unwrap()).contains(&"DIM001"));
}

#[test]
fn rules_scripts_and_files_are_recorded() {
    let model = load(&mini()).unwrap();
    assert!(
        model
            .cube("Sales")
            .unwrap()
            .rules
            .as_ref()
            .unwrap()
            .skipcheck
    );
    assert!(model.cube("Cost").unwrap().rules.is_none());
    let process = &model.processes[0];
    assert!(process.script.contains("sVersion = pVersion;"));
    assert_eq!(
        process.parameters[0].get("Name"),
        Some(&Json::Str("pVersion".into()))
    );
    let names: Vec<&str> = model.files.iter().map(|p| p.name()).collect();
    for name in [
        "tm1project.json",
        "Sales.json",
        "Sales.rules",
        "Load.json",
        "Load.ti",
    ] {
        assert!(names.contains(&name), "{name}");
    }
}

#[test]
fn a_cube_linking_a_missing_rules_file_names_both_paths() {
    let tmp = tempdir();
    let cube_file = tmp.path().join("Ghost.json");
    std::fs::write(&cube_file, r#"{"Name": "Ghost", "Dimensions@Code.links": ["Colour.json"], "Rules@Code.link": "Ghost.rules"}"#).unwrap();
    std::fs::write(
        tmp.path().join("Colour.json"),
        r#"{"Name": "Colour", "Hierarchies@Code.links": ["Colour.hierarchies/Colour.json"]}"#,
    )
    .unwrap();
    let message = model_error(load_cube(&pp(&cube_file), None)).replace('\\', "/");
    assert!(message.contains("Ghost.json") && message.contains("Ghost.rules"));
}

#[test]
fn manifest_refusals_name_the_file() {
    let tmp = tempdir();
    write_manifest(tmp.path(), r#"{"Cubes": ["cubes/Ghost.json"]}"#);
    assert!(
        model_error(load(tmp.path()))
            .replace('\\', "/")
            .contains("cubes/Ghost.json")
    );

    let tmp = tempdir();
    let root = tmp.path().join("model");
    std::fs::write(tmp.path().join("outside.json"), r#"{"Name": "Outside"}"#).unwrap();
    write_manifest(&root, r#"{"Cubes": ["../outside.json"]}"#);
    assert!(model_error(load(&root)).contains("outside the model root"));

    let tmp = tempdir();
    assert!(model_error(load(tmp.path())).contains("tm1project.json"));

    std::fs::write(tmp.path().join("tm1project.json"), "{not json").unwrap();
    assert!(model_error(load(tmp.path())).contains("invalid JSON"));

    std::fs::write(
        tmp.path().join("tm1project.json"),
        format!(r#"{{"Name": {}}}"#, "9".repeat(5000)),
    )
    .unwrap();
    let message = model_error(load(tmp.path()));
    assert!(
        message.contains("invalid JSON") && message.contains("4300 digits"),
        "{message}"
    );

    std::fs::write(tmp.path().join("tm1project.json"), "[".repeat(100_000)).unwrap();
    assert!(model_error(load(tmp.path())).contains("nested too deeply"));
}

const KINDS: [&str; 3] = ["Dimensions", "Cubes", "Processes"];

#[test]
fn manifest_object_collections_must_be_lists() {
    for kind in KINDS {
        for value in [
            "null",
            "true",
            "false",
            "1",
            "1.5",
            r#""""#,
            r#""not-a-list""#,
            "{}",
        ] {
            let tmp = tempdir();
            write_manifest(tmp.path(), &format!(r#"{{"{kind}": {value}}}"#));
            let message = format!("Objects.{kind} must be a list");
            assert!(model_error(load(tmp.path())).contains(&message));
            let (code, out, err) = validate_cli(tmp.path());
            assert_eq!(code, 2);
            assert_eq!(out, "");
            assert!(err.contains(&message));
        }
    }
}

#[test]
fn manifest_link_mappings_and_members_must_be_lists_of_strings() {
    for kind in KINDS {
        let tmp = tempdir();
        let root = write_model(
            tmp.path(),
            &Spec {
                processes: "x = 1;".into(),
                ..Spec::default()
            },
        );
        edit_json(&root.join("tm1project.json"), |manifest| {
            let objects = field(manifest, "Objects");
            let Json::Array(links) = field(objects, kind).clone() else {
                panic!()
            };
            let mapping = links
                .into_iter()
                .map(|l| (l.as_str().unwrap().to_string(), j("{}")))
                .collect();
            set_key(objects, kind, Json::Object(mapping));
        });
        let message = format!("Objects.{kind} must be a list");
        assert!(model_error(load(&root)).contains(&message));
        let (code, out, err) = validate_cli(&root);
        assert!(code == 2 && out.is_empty() && err.contains(&message));

        for member in ["null", "true", "1", "{}", "[]"] {
            let tmp = tempdir();
            write_manifest(tmp.path(), &format!(r#"{{"{kind}": [{member}]}}"#));
            assert!(model_error(load(tmp.path())).contains("link must be a string"));
            let (code, out, err) = validate_cli(tmp.path());
            assert!(code == 2 && out.is_empty() && err.contains("link must be a string"));
        }
    }
}

#[test]
fn absent_and_empty_manifest_collections_remain_valid() {
    for objects in [
        "{}",
        r#"{"Dimensions": [], "Cubes": [], "Processes": []}"#,
        r#"{"Views": null}"#,
        r#"{"Views": 1}"#,
    ] {
        let tmp = tempdir();
        write_manifest(tmp.path(), objects);
        let model = load(tmp.path()).unwrap();
        assert!(
            model.dimensions.is_empty() && model.cubes.is_empty() && model.processes.is_empty()
        );
        let (code, _, err) = validate_cli(tmp.path());
        assert_eq!(code, 0);
        assert_eq!(err, "");
    }
}

#[test]
fn nested_link_values_use_the_same_model_error() {
    let cases: [(&str, &str, &str); 4] = [
        ("dimensions/Colour.json", "Hierarchies@Code.links", "[null]"),
        ("cubes/Sales.json", "Dimensions@Code.links", "[null]"),
        ("cubes/Sales.json", "Rules@Code.link", "true"),
        ("processes/Load.json", "Code@Code.link", "true"),
    ];
    for (relative, key, value) in cases {
        let tmp = tempdir();
        let root = write_model(
            tmp.path(),
            &Spec {
                processes: "x = 1;".into(),
                ..Spec::default()
            },
        );
        let path = root.join(relative);
        edit_json(&path, |p| set_key(p, key, j(value)));
        let standalone = match relative {
            "dimensions/Colour.json" => load_dimension(&pp(&path), None).map(|_| ()),
            "processes/Load.json" => load_process(&pp(&path), None).map(|_| ()),
            _ => load_cube(&pp(&path), None).map(|_| ()),
        };
        for result in [standalone, load(&root).map(|_| ())] {
            let message = model_error(result);
            assert!(
                message.contains("link must be a string") && message.contains(&s(&pp(&path).fs())),
                "{message}"
            );
        }
        let (code, out, err) = validate_cli(&root);
        assert!(code == 2 && out.is_empty() && err.contains(&pp(&path).display()));
    }
}

#[test]
fn hierarchy_link_collection_must_be_a_list() {
    for value in [
        "false",
        "true",
        "0",
        "1",
        "1.5",
        r#""""#,
        r#""Colour.hierarchies/Colour.json""#,
        "{}",
        r#"{"Colour.hierarchies/Colour.json": false}"#,
    ] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        let path = root.join("dimensions").join("Colour.json");
        edit_json(&path, |p| set_key(p, "Hierarchies@Code.links", j(value)));
        for result in [
            load_dimension(&pp(&path), None).map(|_| ()),
            load(&root).map(|_| ()),
        ] {
            let message = model_error(result);
            assert!(
                message.contains("Hierarchies@Code.links must be a list")
                    && message.contains(&pp(&path).display())
            );
        }
        let (code, out, err) = validate_cli(&root);
        assert!(
            code == 2 && out.is_empty() && err.contains("Hierarchies@Code.links must be a list")
        );
    }
}

#[test]
fn falsey_optional_link_values_must_be_strings() {
    for (relative, key) in [
        ("cubes/Sales.json", "Rules@Code.link"),
        ("processes/Load.json", "Code@Code.link"),
    ] {
        for value in ["false", "0", "0.0", "[]", "{}"] {
            let tmp = tempdir();
            let process = relative.starts_with("processes");
            let root = write_model(
                tmp.path(),
                &Spec {
                    processes: if process {
                        "x = 1;".into()
                    } else {
                        String::new()
                    },
                    ..Spec::default()
                },
            );
            if process {
                std::fs::remove_file(root.join("processes").join("Load.ti")).unwrap();
            }
            let path = root.join(relative);
            edit_json(&path, |p| set_key(p, key, j(value)));
            let standalone = if process {
                load_process(&pp(&path), None).map(|_| ())
            } else {
                load_cube(&pp(&path), None).map(|_| ())
            };
            for result in [standalone, load(&root).map(|_| ())] {
                let message = model_error(result);
                assert!(
                    message.contains("link must be a string")
                        && message.contains(&pp(&path).display())
                );
            }
            let (code, out, err) = validate_cli(&root);
            assert!(code == 2 && out.is_empty() && err.contains("link must be a string"));
        }
    }
}

#[test]
fn optional_links_preserve_no_source_states() {
    for state in ["omitted", "null", "empty"] {
        let tmp = tempdir();
        let root = write_model(
            tmp.path(),
            &Spec {
                processes: "x = 1;".into(),
                ..Spec::default()
            },
        );
        std::fs::remove_file(root.join("processes").join("Load.ti")).unwrap();
        for (relative, key) in [
            ("cubes/Sales.json", "Rules@Code.link"),
            ("processes/Load.json", "Code@Code.link"),
        ] {
            edit_json(&root.join(relative), |p| match state {
                "omitted" => remove_key(p, key),
                "null" => set_key(p, key, Json::Null),
                _ => set_key(p, key, Json::Str(String::new())),
            });
        }
        let cube = load_cube(&pp(&root.join("cubes/Sales.json")), None).unwrap();
        let process = load_process(&pp(&root.join("processes/Load.json")), None).unwrap();
        assert!(cube.rules.is_none() && cube.rules_source.is_none());
        assert!(process.script.is_empty() && process.script_source.is_none());
        let model = load(&root).unwrap();
        let names: Vec<&str> = model.files.iter().map(|p| p.name()).collect();
        assert!(!names.contains(&"Sales.rules") && !names.contains(&"Load.ti"));
        assert_eq!(
            validate_cli(&root),
            (0, "0 errors, 0 warnings\n".into(), String::new())
        );
    }
}

#[test]
fn hierarchy_links_preserve_no_hierarchy_error() {
    for state in ["omitted", "null", "empty"] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        let path = root.join("dimensions/Colour.json");
        edit_json(&path, |p| match state {
            "omitted" => remove_key(p, "Hierarchies@Code.links"),
            "null" => set_key(p, "Hierarchies@Code.links", Json::Null),
            _ => set_key(p, "Hierarchies@Code.links", j("[]")),
        });
        for result in [
            load_dimension(&pp(&path), None).map(|_| ()),
            load(&root).map(|_| ()),
        ] {
            assert!(model_error(result).contains("links no hierarchy file"));
        }
    }
}

#[test]
fn invalid_nested_link_shape_exits_the_cli_process() {
    for (relative, key, value, message) in [
        (
            "dimensions/Colour.json",
            "Hierarchies@Code.links",
            "true",
            "Hierarchies@Code.links must be a list",
        ),
        (
            "cubes/Sales.json",
            "Rules@Code.link",
            "false",
            "link must be a string",
        ),
    ] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        let path = root.join(relative);
        edit_json(&path, |p| set_key(p, key, j(value)));
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_pacioliscube"))
            .args(["validate", &s(&root)])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let err = String::from_utf8(output.stderr).unwrap();
        assert!(err.contains(message) && err.contains(&pp(&path).display()));
    }
}

fn add_alias(
    root: &Path,
    kind: &str,
    same_path: bool,
    name: Option<&str>,
    reverse: bool,
) -> (PathBuf, PathBuf) {
    let original = root.join(match kind {
        "Dimensions" => "dimensions/Colour.json",
        "Cubes" => "cubes/Sales.json",
        _ => "processes/Load.json",
    });
    let duplicate = if same_path {
        original.clone()
    } else {
        original.with_file_name("Alternate.json")
    };
    if !same_path {
        std::fs::copy(&original, &duplicate).unwrap();
        if let Some(name) = name {
            edit_json(&duplicate, |p| set_key(p, "Name", Json::Str(name.into())));
        }
    }
    let relative = duplicate
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    edit_json(&root.join("tm1project.json"), |manifest| {
        let list = field(field(manifest, "Objects"), kind);
        push(list, Json::Str(relative));
        if reverse {
            let Json::Array(items) = list else { panic!() };
            items.reverse();
        }
    });
    (original, duplicate)
}

#[test]
fn duplicate_object_names_are_rejected_before_they_are_lost() {
    for kind in KINDS {
        for same_path in [false, true] {
            for reverse in [false, true] {
                let tmp = tempdir();
                let root = write_model(
                    tmp.path(),
                    &Spec {
                        processes: "x = 1;".into(),
                        ..Spec::default()
                    },
                );
                let (original, duplicate) = add_alias(&root, kind, same_path, None, reverse);
                let message = model_error(load(&root));
                let resolved = |p: &Path| pp(p).resolve().unwrap().display();
                assert!(message.contains(&resolved(&original)), "{message}");
                assert!(message.contains(&resolved(&duplicate)), "{message}");
                let name = match kind {
                    "Dimensions" => "Colour",
                    "Cubes" => "Sales",
                    _ => "Load",
                };
                assert!(message.contains(name));
            }
        }
    }
}

#[test]
fn cube_names_must_be_distinct_under_case_folding() {
    for reverse in [false, true] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        add_alias(&root, "Cubes", false, Some("sales"), reverse);
        let message = model_error(load(&root));
        assert!(message.contains("Sales") && message.contains("sales"));
    }
    let tmp = tempdir();
    let root = write_model(tmp.path(), &Spec::default());
    edit_json(&root.join("cubes/Sales.json"), |p| {
        set_key(p, "Name", Json::Str("Straße".into()))
    });
    add_alias(&root, "Cubes", false, Some("STRASSE"), false);
    let message = model_error(load(&root));
    assert!(message.contains("Straße") && message.contains("STRASSE"));
}

#[test]
fn model_names_must_be_text_at_the_loading_boundary() {
    for name in ["1", "true", r#"["Sales"]"#, r#"{"name": "Sales"}"#] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        let path = root.join("cubes/Sales.json");
        edit_json(&path, |p| set_key(p, "Name", j(name)));
        let message = model_error(load_cube(&pp(&path), None));
        assert!(
            message.contains("Name must be a string") && message.contains(&pp(&path).display())
        );
    }
    let fields: [(&str, &[&str]); 6] = [
        ("dimensions/Colour.json", &["Name"]),
        ("dimensions/Colour.hierarchies/Colour.json", &["Name"]),
        (
            "dimensions/Colour.hierarchies/Colour.json",
            &["Elements", "0", "Name"],
        ),
        (
            "dimensions/Colour.hierarchies/Colour.json",
            &["Edges", "0", "ParentName"],
        ),
        (
            "dimensions/Colour.hierarchies/Colour.json",
            &["Edges", "0", "ComponentName"],
        ),
        ("processes/Load.json", &["Name"]),
    ];
    for (relative, keys) in fields {
        for name in ["42", "true", r#"["bad-name"]"#, r#"{"name": "bad-name"}"#] {
            let tmp = tempdir();
            let root = write_model(
                tmp.path(),
                &Spec {
                    processes: "x = 1;".into(),
                    ..Spec::default()
                },
            );
            let path = root.join(relative);
            edit_json(&path, |p| {
                let mut item = p;
                for key in &keys[..keys.len() - 1] {
                    item = match item {
                        Json::Array(items) => &mut items[key.parse::<usize>().unwrap()],
                        other => field(other, key),
                    };
                }
                set_key(item, keys[keys.len() - 1], j(name));
            });
            let message = model_error(load(&root));
            assert!(message.contains("must be"), "{message}");
            assert!(
                message.contains(&pp(&path).resolve().unwrap().display()),
                "{message}"
            );
            assert!(message.contains(keys[keys.len() - 1]), "{message}");
        }
    }
}

#[test]
fn a_standalone_cube_reads_linked_dimension_names() {
    for name in ["42", "true", r#"["bad-name"]"#, r#"{"name": "bad-name"}"#] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        edit_json(&root.join("dimensions/Colour.json"), |p| {
            set_key(p, "Name", j(name))
        });
        let message = model_error(load_cube(&pp(&root.join("cubes/Sales.json")), None));
        assert!(message.contains("Name must be a string"));
    }
    for name in ["null", r#""""#, "0", "false", "[]", "{}"] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        edit_json(&root.join("dimensions/Colour.json"), |p| {
            set_key(p, "Name", j(name))
        });
        let cube = load_cube(&pp(&root.join("cubes/Sales.json")), None).unwrap();
        assert_eq!(cube.dimensions, ["Colour", "Measure"]);
    }
    for name in ["Straße", " Colour "] {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        edit_json(&root.join("dimensions/Colour.json"), |p| {
            set_key(p, "Name", Json::Str(name.into()))
        });
        let cube = load_cube(&pp(&root.join("cubes/Sales.json")), None).unwrap();
        assert_eq!(cube.dimensions, [name, "Measure"]);
    }
}

#[test]
fn cli_reports_non_text_names_as_invalid_models() {
    let cases: [(&str, &[&str], &str); 3] = [
        ("processes/Load.json", &["Name"], "42"),
        ("dimensions/Colour.json", &["Name"], r#"["bad-name"]"#),
        (
            "dimensions/Colour.hierarchies/Colour.json",
            &["Edges", "0", "ParentName"],
            "42",
        ),
    ];
    for command in ["validate", "calculate"] {
        for (relative, keys, name) in cases {
            let tmp = tempdir();
            let root = write_model(
                &tmp.path().join("model"),
                &Spec {
                    processes: "x = 1;".into(),
                    ..Spec::default()
                },
            );
            let path = root.join(relative);
            edit_json(&path, |p| {
                let mut item = p;
                for key in &keys[..keys.len() - 1] {
                    item = match item {
                        Json::Array(items) => &mut items[key.parse::<usize>().unwrap()],
                        other => field(other, key),
                    };
                }
                set_key(item, keys[keys.len() - 1], j(name));
            });
            let data = build_data(&tmp.path().join("data"), "sales.csv", "Red,Units,6\n");
            let (code, out, err) = if command == "validate" {
                run(&["validate", &s(&root)])
            } else {
                run(&[
                    "calculate",
                    &s(&root),
                    "--data",
                    &data,
                    "--cell",
                    "Sales:Red,Units",
                ])
            };
            assert_eq!(code, 2);
            assert_eq!(out, "");
            assert!(
                err.contains(&pp(&path).resolve().unwrap().display())
                    || err.contains(&pp(&path).display())
            );
        }
    }
}

#[test]
fn object_names_in_different_kinds_keep_separate_namespaces() {
    let tmp = tempdir();
    let root = write_model(
        tmp.path(),
        &Spec {
            processes: "x = 1;".into(),
            ..Spec::default()
        },
    );
    for relative in ["cubes/Sales.json", "processes/Load.json"] {
        edit_json(&root.join(relative), |p| {
            set_key(p, "Name", Json::Str("Colour".into()))
        });
    }
    let model = load(&root).unwrap();
    assert!(model.has_dimension("Colour"));
    assert!(model.cube("Colour").is_some());
    assert_eq!(model.processes[0].name, "Colour");

    for (kind, name) in [("Dimensions", "colour"), ("Processes", "load")] {
        let tmp = tempdir();
        let root = write_model(
            tmp.path(),
            &Spec {
                processes: "x = 1;".into(),
                ..Spec::default()
            },
        );
        add_alias(&root, kind, false, Some(name), false);
        let model = load(&root).unwrap();
        if kind == "Dimensions" {
            assert!(model.has_dimension(name) && model.has_dimension("Colour"));
        } else {
            let names: Vec<&str> = model.processes.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, ["Load", "load"]);
        }
    }
}

#[test]
fn cli_rejects_ambiguous_cube_names_without_printing_a_result() {
    for name in ["Sales", "sales"] {
        for command in ["validate", "calculate"] {
            for reverse in [false, true] {
                let tmp = tempdir();
                let root = write_model(
                    &tmp.path().join("model"),
                    &Spec::rules(
                        "SKIPCHECK; ['Amount'] = N: ['Units'] * ['Price']; FEEDERS; ['Units'] => ['Amount'];",
                    ),
                );
                let (original, duplicate) = add_alias(&root, "Cubes", false, Some(name), reverse);
                edit_json(&duplicate, |p| {
                    set_key(p, "Rules@Code.link", Json::Str("Alternate.rules".into()))
                });
                std::fs::write(duplicate.with_extension("rules"), "SKIPCHECK; ['Amount'] = N: ['Units'] * ['Price'] * 2; FEEDERS; ['Units'] => ['Amount'];").unwrap();
                let data = build_data(
                    &tmp.path().join("data"),
                    "sales.csv",
                    "Red,Units,6\nRed,Price,4\n",
                );
                let (code, out, err) = if command == "validate" {
                    run(&["validate", &s(&root)])
                } else {
                    run(&[
                        "calculate",
                        &s(&root),
                        "--data",
                        &data,
                        "--cell",
                        "Sales:Red,Amount",
                    ])
                };
                assert_eq!(code, 2);
                assert_eq!(out, "");
                let resolved = |p: &Path| pp(p).resolve().unwrap().display();
                assert!(
                    err.contains(&resolved(&original)) && err.contains(&resolved(&duplicate)),
                    "{err}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- dimensions

fn write_dimension(root: &Path, name: &str, elements: &str, edges: &str) -> PathBuf {
    let file = root.join(format!("{name}.json"));
    std::fs::write(
        &file,
        format!(
            r#"{{"Name": "{name}", "Hierarchies@Code.links": ["{name}.hierarchies/{name}.json"]}}"#
        ),
    )
    .unwrap();
    let dir = root.join(format!("{name}.hierarchies"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.json")),
        format!(r#"{{"Name": "{name}", "Elements": [{elements}], "Edges": [{edges}]}}"#),
    )
    .unwrap();
    file
}

fn colour() -> pacioliscube::model::Dimension {
    load_dimension(&pp(&mini().join("dimensions").join("Colour.json")), None).unwrap()
}

#[test]
fn loads_elements_edges_and_weights() {
    let dimension = colour();
    let hierarchy = dimension.default_hierarchy().unwrap();
    assert_eq!(dimension.name, "Colour");
    let mut names: Vec<&str> = hierarchy.elements.iter().map(|e| e.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Blue", "Contra", "Red", "Total"]);
    assert_eq!(hierarchy.elements[0].element_type, "Consolidated");
    assert!(hierarchy.is_leaf("Red").unwrap());
    assert!(!hierarchy.is_leaf("Total").unwrap());
    let weights: Vec<(String, String)> = hierarchy
        .children("Total")
        .unwrap()
        .iter()
        .map(|e| (e.component.clone(), e.weight.to_sci_string()))
        .collect();
    assert_eq!(
        weights,
        [
            ("Red".to_string(), "1".to_string()),
            ("Blue".into(), "1".into()),
            ("Contra".into(), "-1".into())
        ]
    );
    assert!(hierarchy.is_leaf("red").unwrap());
    assert_eq!(hierarchy.resolve("rED").unwrap(), "Red");
    assert!(
        hierarchy
            .resolve("Green")
            .unwrap_err()
            .message()
            .contains("Green")
    );
    assert_eq!(
        hierarchy.leaves_under("Total").unwrap(),
        ["Red", "Blue", "Contra"]
    );
    assert_eq!(hierarchy.leaves_under("Red").unwrap(), ["Red"]);
}

#[test]
fn hierarchy_entries_and_collections_are_checked() {
    for collection in ["Elements", "Edges"] {
        for invalid in ["null", "12", r#""name""#, "[]"] {
            let tmp = tempdir();
            let (elements, edges) = if collection == "Elements" {
                (invalid, "")
            } else {
                ("", invalid)
            };
            let path = write_dimension(tmp.path(), "Test", elements, edges);
            let message = model_error(load_dimension(&pp(&path), None));
            assert!(message.contains("expected an object") && message.contains("Test.hierarchies"));
        }
        for invalid in ["null", "12", r#""entries""#, "{}", "true"] {
            let tmp = tempdir();
            let path = write_dimension(tmp.path(), "Test", "", "");
            let source = tmp.path().join("Test.hierarchies").join("Test.json");
            edit_json(&source, |p| set_key(p, collection, j(invalid)));
            let message = model_error(load_dimension(&pp(&path), None));
            assert!(
                message.contains(&format!("{collection} must be a list"))
                    && message.contains(&pp(&source).resolve().unwrap().display())
            );
        }
    }
    for omitted in [false, true] {
        let tmp = tempdir();
        let path = write_dimension(tmp.path(), "Test", "", "");
        if omitted {
            std::fs::write(
                tmp.path().join("Test.hierarchies").join("Test.json"),
                r#"{"Name": "Test"}"#,
            )
            .unwrap();
        }
        let dimension = load_dimension(&pp(&path), None).unwrap();
        let hierarchy = dimension.default_hierarchy().unwrap();
        assert!(hierarchy.elements.is_empty() && hierarchy.edges.is_empty());
    }
    for invalid in ["[]", "{}", "null", "12", "true"] {
        let tmp = tempdir();
        let path = write_dimension(tmp.path(), "Test", r#"{"Name": "Leaf"}"#, "");
        let source = tmp.path().join("Test.hierarchies").join("Test.json");
        edit_json(&source, |p| {
            let Json::Array(items) = field(p, "Elements") else {
                panic!()
            };
            set_key(&mut items[0], "Type", j(invalid));
        });
        assert!(model_error(load_dimension(&pp(&path), None)).contains("has type"));
    }
}

fn share(weight: &str) -> Result<pacioliscube::model::Dimension, Error> {
    let tmp = tempdir();
    let path = write_dimension(
        tmp.path(),
        "Share",
        r#"{"Name": "Total", "Type": "Consolidated"}, {"Name": "Part", "Type": "Numeric"}"#,
        &format!(r#"{{"ParentName": "Total", "ComponentName": "Part", "Weight": {weight}}}"#),
    );
    load_dimension(&pp(&path), None)
}

#[test]
fn weights_keep_precision_and_refuse_non_finite_values() {
    let weight = |w: &str| {
        share(w)
            .unwrap()
            .default_hierarchy()
            .unwrap()
            .children("Total")
            .unwrap()[0]
            .weight
            .clone()
    };
    assert_eq!(weight("0.1").to_sci_string(), "0.1");
    for (w, expected) in [
        ("0", "0"),
        ("-1", "-1"),
        ("-0.25", "-0.25"),
        (r#""2""#, "2"),
    ] {
        assert_eq!(weight(w), d(expected));
    }
    for w in [
        r#""NaN""#,
        r#""sNaN""#,
        r#""Infinity""#,
        r#""-Infinity""#,
        "NaN",
        "Infinity",
        "-Infinity",
    ] {
        let message = model_error(share(w));
        assert!(
            message.contains("is not a finite number") && message.contains("Total to Part"),
            "{message}"
        );
    }
}

#[test]
fn shared_subtrees_report_leaves_once_in_edge_order() {
    let tmp = tempdir();
    let path = write_dimension(
        tmp.path(),
        "Shared",
        r#"{"Name": "Top", "Type": "Consolidated"}, {"Name": "Left", "Type": "Consolidated"}, {"Name": "Right", "Type": "Consolidated"}, {"Name": "Leaf", "Type": "Numeric"}"#,
        r#"{"ParentName": "Top", "ComponentName": "Left", "Weight": 1}, {"ParentName": "Top", "ComponentName": "Right", "Weight": 1}, {"ParentName": "Left", "ComponentName": "Leaf", "Weight": 1}, {"ParentName": "Right", "ComponentName": "Leaf", "Weight": 1}"#,
    );
    let dimension = load_dimension(&pp(&path), None).unwrap();
    assert_eq!(
        dimension
            .default_hierarchy()
            .unwrap()
            .leaves_under("Top")
            .unwrap(),
        ["Leaf"]
    );

    // Twelve layers of two nodes, each linked to both nodes below: without
    // visiting each shared subtree once, this would expand 2 ** 12 paths.
    let layers: Vec<[String; 2]> = (0..12)
        .map(|l| [format!("Left{l}"), format!("Right{l}")])
        .collect();
    let mut elements = vec![r#"{"Name": "Top", "Type": "Consolidated"}"#.to_string()];
    for layer in &layers {
        for name in layer {
            elements.push(format!(r#"{{"Name": "{name}", "Type": "Consolidated"}}"#));
        }
    }
    elements.push(r#"{"Name": "First", "Type": "Numeric"}"#.into());
    elements.push(r#"{"Name": "Second", "Type": "Numeric"}"#.into());
    let edge = |p: &str, c: &str| {
        format!(r#"{{"ParentName": "{p}", "ComponentName": "{c}", "Weight": 1}}"#)
    };
    let mut edges: Vec<String> = layers[0].iter().map(|c| edge("Top", c)).collect();
    for pair in layers.windows(2) {
        for parent in &pair[0] {
            for child in &pair[1] {
                edges.push(edge(parent, child));
            }
        }
    }
    edges.push(edge(&layers[11][0], "First"));
    edges.push(edge(&layers[11][0], "Second"));
    edges.push(edge(&layers[11][1], "Second"));
    edges.push(edge(&layers[11][1], "First"));
    let tmp = tempdir();
    let path = write_dimension(tmp.path(), "Shared", &elements.join(","), &edges.join(","));
    let dimension = load_dimension(&pp(&path), None).unwrap();
    assert_eq!(
        dimension
            .default_hierarchy()
            .unwrap()
            .leaves_under("top")
            .unwrap(),
        ["First", "Second"]
    );
}

#[test]
fn broken_hierarchies_are_model_errors() {
    let tmp = tempdir();
    let path = write_dimension(
        tmp.path(),
        "Broken",
        r#"{"Name": "A", "Type": "Numeric"}"#,
        r#"{"ParentName": "A", "ComponentName": "Ghost", "Weight": 1}"#,
    );
    let message = model_error(load_dimension(&pp(&path), None)).replace('\\', "/");
    assert!(message.contains("Ghost") && message.contains("Broken.json"));

    let tmp = tempdir();
    let path = write_dimension(
        tmp.path(),
        "Loop",
        r#"{"Name": "A", "Type": "Consolidated"}, {"Name": "B", "Type": "Consolidated"}"#,
        r#"{"ParentName": "A", "ComponentName": "B", "Weight": 1}, {"ParentName": "B", "ComponentName": "A", "Weight": 1}"#,
    );
    let message = model_error(load_dimension(&pp(&path), None));
    assert!(
        message.ends_with("hierarchy contains a cycle, A -> B -> A"),
        "{message}"
    );

    let tmp = tempdir();
    let file = tmp.path().join("Absent.json");
    std::fs::write(
        &file,
        r#"{"Name": "Absent", "Hierarchies@Code.links": ["Absent.hierarchies/Absent.json"]}"#,
    )
    .unwrap();
    assert!(
        model_error(load_dimension(&pp(&file), None))
            .replace('\\', "/")
            .contains("Absent.hierarchies/Absent.json")
    );
}

// ---------------------------------------------------------------- loader refusals

#[test]
fn a_malformed_model_tree_is_refused_with_its_reason() {
    type Change = fn(&Path);
    let cases: [(Change, &str); 13] = [
        (
            |r| std::fs::write(r.join("tm1project.json"), "[]").unwrap(),
            "expected a JSON object at the top level",
        ),
        (
            |r| {
                edit_json(&colour_hierarchy(r), |h| {
                    push(field(h, "Elements"), j(r#"{"Name": "red"}"#))
                })
            },
            "element 'red' is declared twice",
        ),
        (
            |r| edit_json(&colour_hierarchy(r), |h| remove_key(h, "Name")),
            "hierarchy has no Name",
        ),
        (
            |r| {
                edit_json(&colour_hierarchy(r), |h| {
                    push(field(h, "Elements"), j(r#"{"Type": "Numeric"}"#))
                })
            },
            "an element has no Name",
        ),
        (
            |r| {
                edit_json(&colour_hierarchy(r), |h| {
                    push(
                        field(h, "Elements"),
                        j(r#"{"Name": "Pink", "Type": "Text"}"#),
                    )
                })
            },
            "element 'Pink' has type 'Text'",
        ),
        (
            |r| {
                edit_json(&colour_hierarchy(r), |h| {
                    push(field(h, "Edges"), j(r#"{"ParentName": "Total"}"#))
                })
            },
            "an edge is missing ParentName or ComponentName",
        ),
        (
            |r| edit_json(&r.join("dimensions/Colour.json"), |d| remove_key(d, "Name")),
            "dimension has no Name",
        ),
        (
            |r| {
                edit_json(&r.join("dimensions/Colour.json"), |d| {
                    push(
                        field(d, "Hierarchies@Code.links"),
                        Json::Str("Colour.hierarchies/Colour.json".into()),
                    )
                })
            },
            "hierarchy 'Colour' is linked twice",
        ),
        (
            |r| edit_json(&r.join("cubes/Sales.json"), |c| remove_key(c, "Name")),
            "cube has no Name",
        ),
        (
            |r| {
                edit_json(&r.join("cubes/Sales.json"), |c| {
                    push(
                        field(c, "Dimensions@Code.links"),
                        Json::Str("../dimensions/Missing.json".into()),
                    )
                })
            },
            "linked dimension file '../dimensions/Missing.json' not found",
        ),
        (
            |r| {
                edit_json(&r.join("tm1project.json"), |m| {
                    set_key(m, "Objects", j("[]"))
                })
            },
            "Objects must be an object",
        ),
        (
            |r| {
                edit_json(&r.join("tm1project.json"), |m| {
                    push(
                        field(field(m, "Objects"), "Dimensions"),
                        Json::Str("dimensions/Missing.json".into()),
                    )
                })
            },
            "lists 'dimensions/Missing.json', which is not on disk",
        ),
        (
            |r| {
                edit_json(&r.join("tm1project.json"), |m| {
                    set_key(
                        field(m, "Objects"),
                        "Processes",
                        j(r#"["processes/Missing.json"]"#),
                    )
                })
            },
            "lists 'processes/Missing.json', which is not on disk",
        ),
    ];
    for (change, message) in cases {
        let tmp = tempdir();
        let root = write_model(tmp.path(), &Spec::default());
        change(&root);
        let found = model_error(load(&root));
        assert!(found.contains(message), "{found} / {message}");
    }
}

#[test]
fn a_malformed_process_is_refused() {
    let tmp = tempdir();
    let root = write_model(
        tmp.path(),
        &Spec {
            processes: "#****Begin: Prolog script****\n".into(),
            ..Spec::default()
        },
    );
    edit_json(&root.join("processes/Load.json"), |p| remove_key(p, "Name"));
    assert!(model_error(load(&root)).contains("process has no Name"));
    let tmp = tempdir();
    let root = write_model(
        tmp.path(),
        &Spec {
            processes: "#****Begin: Prolog script****\n".into(),
            ..Spec::default()
        },
    );
    edit_json(&root.join("processes/Load.json"), |p| {
        set_key(p, "Code@Code.link", Json::Str("Missing.ti".into()))
    });
    assert!(
        model_error(load(&root))
            .contains("process 'Load' links script 'Missing.ti', which is missing")
    );
}

#[test]
fn default_hierarchies_fall_back_or_refuse() {
    let tmp = tempdir();
    let root = write_model(tmp.path(), &Spec::default());
    edit_json(&colour_hierarchy(&root), |h| {
        set_key(h, "Name", Json::Str("Alternate".into()))
    });
    let dimension = load_dimension(&pp(&root.join("dimensions/Colour.json")), None).unwrap();
    assert_eq!(dimension.default_hierarchy().unwrap().name, "Alternate");

    let tmp = tempdir();
    let root = write_model(tmp.path(), &Spec::default());
    let dir = root.join("dimensions").join("Colour.hierarchies");
    edit_json(&dir.join("Colour.json"), |h| {
        set_key(h, "Name", Json::Str("First".into()))
    });
    std::fs::copy(dir.join("Colour.json"), dir.join("Second.json")).unwrap();
    edit_json(&dir.join("Second.json"), |h| {
        set_key(h, "Name", Json::Str("Second".into()))
    });
    edit_json(&root.join("dimensions/Colour.json"), |d| {
        push(
            field(d, "Hierarchies@Code.links"),
            Json::Str("Colour.hierarchies/Second.json".into()),
        )
    });
    let dimension = load_dimension(&pp(&root.join("dimensions/Colour.json")), None).unwrap();
    assert!(
        dimension
            .default_hierarchy()
            .unwrap_err()
            .message()
            .contains("no hierarchy named 'Colour' to use as the default")
    );

    let model = load(&write_model(tempdir().path(), &Spec::default())).unwrap();
    assert!(
        model
            .hierarchy("Region")
            .unwrap_err()
            .message()
            .contains("no dimension named 'Region'")
    );
}

#[test]
fn data_loader_refusals() {
    let tmp = tempdir();
    let model = load(&write_model(&tmp.path().join("model"), &Spec::default())).unwrap();
    let cube = model.cube("Sales").unwrap();
    assert!(
        model_error(read_csv(&pp(&tmp.path().join("missing.csv")), cube, &model))
            .contains("file not found")
    );
    let empty = tmp.path().join("empty.csv");
    std::fs::write(&empty, "").unwrap();
    assert!(
        model_error(read_csv(&pp(&empty), cube, &model))
            .contains("the file is empty, expected a header row")
    );
    let mut store = pacioliscube::evaluate::CellStore::new();
    assert!(
        model_error(pacioliscube::data::load_into_store(
            &model,
            "Budget",
            &pp(&empty),
            &mut store
        ))
        .contains("no cube named 'Budget'")
    );
    let refused = pacioliscube::data::load_data(&model, &pp(&tmp.path().join("no-such-directory")))
        .unwrap_err();
    assert!(
        matches!(&refused, Error::Cli(1, m) if m.contains("the data directory could not be read")),
        "{refused:?}"
    );
}

// ---------------------------------------------------------------- shipped content

fn shipped() -> Model {
    load(&model_root()).unwrap()
}

#[test]
fn the_model_ships_every_dimension() {
    let model = shipped();
    let mut names: Vec<&str> = shipped_names(&model);
    names.sort();
    let mut expected = [
        "Year",
        "Period",
        "Version",
        "Entity",
        "CostCentre",
        "Account",
        "Role",
        "Fleet",
        "DriverMeasure",
        "WorkforceMeasure",
        "RevenueMeasure",
        "CapexMeasure",
        "PnLMeasure",
    ];
    expected.sort();
    assert_eq!(names, expected);
}

fn shipped_names(model: &Model) -> Vec<&str> {
    model.dimensions.iter().map(|d| d.name.as_str()).collect()
}

#[test]
fn shipped_hierarchies_have_the_expected_shape() {
    let model = shipped();
    let period = model.hierarchy("Period").unwrap();
    let fy = period.leaves_under("FY").unwrap();
    assert_eq!(fy.len(), 12);
    assert_eq!((fy[0], fy[11]), ("Jul", "Jun"));
    assert_eq!(period.children("FY").unwrap().len(), 4);
    assert_eq!(period.leaves_under("Q1").unwrap(), ["Jul", "Aug", "Sep"]);
    let entity = model.hierarchy("Entity").unwrap();
    let mut members: Vec<&str> = entity
        .children("Group")
        .unwrap()
        .iter()
        .map(|e| e.component.as_str())
        .collect();
    members.sort();
    assert_eq!(members, ["CivilCo", "HaulCo"]);
    let account = model.hierarchy("Account").unwrap();
    let ebit = account.elements.iter().find(|e| e.name == "EBIT").unwrap();
    assert_eq!(ebit.element_type, "Consolidated");
    let weight = |parent: &str, child: &str| {
        account
            .children(parent)
            .unwrap()
            .iter()
            .find(|e| e.component == child)
            .unwrap()
            .weight
            .clone()
    };
    assert_eq!(weight("Gross Margin", "Revenue"), d("1"));
    assert_eq!(weight("Gross Margin", "Direct Costs"), d("-1"));
    assert_eq!(weight("EBIT", "Depreciation"), d("-1"));
    let mut reachable: Vec<&str> = account.leaves_under("EBIT").unwrap();
    reachable.sort();
    let mut leaves: Vec<&str> = account
        .elements
        .iter()
        .filter(|e| account.is_leaf(&e.name).unwrap())
        .map(|e| e.name.as_str())
        .collect();
    leaves.sort();
    assert_eq!(reachable, leaves);
}

// ---------------------------------------------------------------- TM1 source form

fn sorted_files(folder: &str, suffix: &str) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(model_root().join(folder))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no {suffix} under model/{folder}");
    paths
}

#[test]
fn every_cube_names_its_dimensions_as_the_specification_shows() {
    for path in sorted_files("cubes", ".json") {
        let payload = j(&std::fs::read_to_string(&path).unwrap());
        assert!(!payload.has("Dimensions@Code.links"), "{path:?}");
        let Some(Json::Array(entries)) = payload.get("Dimensions") else {
            panic!("{path:?}")
        };
        assert!(!entries.is_empty());
        for entry in entries {
            assert!(
                dimension_id(entry.get("@id").unwrap().as_str().unwrap()).is_some(),
                "{path:?}"
            );
        }
    }
}

#[test]
fn every_process_declares_security_access_and_links_its_code() {
    for path in sorted_files("processes", ".json") {
        let payload = j(&std::fs::read_to_string(&path).unwrap());
        assert!(
            matches!(payload.get("HasSecurityAccess"), Some(Json::Bool(_))),
            "{path:?}"
        );
        let stem = path.file_stem().unwrap().to_string_lossy();
        assert_eq!(
            payload.get("Code@Code.link").and_then(Json::as_str),
            Some(format!("{stem}.ti").as_str())
        );
    }
}

#[test]
fn every_script_has_the_four_procedure_regions_in_server_order() {
    let markers: Vec<String> = ["Prolog", "Metadata", "Data", "Epilog"]
        .iter()
        .flat_map(|n| [format!("#region {n}"), "#endregion".to_string()])
        .collect();
    let is_region = |line: &str| {
        let lower = line.to_ascii_lowercase();
        ["#region", "#endregion"].iter().any(|m| {
            lower.starts_with(m)
                && lower[m.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !py::is_word(c))
        })
    };
    for path in sorted_files("processes", ".ti") {
        let text = std::fs::read_to_string(&path).unwrap();
        let lines = py::splitlines(&text);
        assert_eq!(lines[0], "#region Prolog", "{path:?}");
        let found: Vec<&str> = lines.iter().copied().filter(|l| is_region(l)).collect();
        assert_eq!(found, markers, "{path:?}");
    }
}

/// tm1gitpy's reading of the shipped model, written by the CI job that installs
/// it (ci/tm1gitpy_model.py), must agree with this engine's.
#[test]
fn tm1gitpy_reads_the_model_the_way_pacioliscube_does() {
    let Ok(path) = std::env::var("PACIOLISCUBE_TM1GITPY_JSON") else {
        assert_ne!(
            std::env::var("PACIOLISCUBE_REQUIRE_TM1GITPY").as_deref(),
            Ok("1"),
            "tm1gitpy output is required"
        );
        return;
    };
    let theirs = j(&std::fs::read_to_string(path).unwrap());
    assert_eq!(theirs.get("errors"), Some(&Json::Object(vec![])));
    let ours = shipped();
    let Some(Json::Array(dimensions)) = theirs.get("dimensions") else {
        panic!()
    };
    let mut names: Vec<&str> = dimensions
        .iter()
        .map(|d| d.get("name").unwrap().as_str().unwrap())
        .collect();
    names.sort();
    let mut mine = shipped_names(&ours);
    mine.sort();
    assert_eq!(names, mine);
    for dimension in dimensions {
        let Some(Json::Array(hierarchies)) = dimension.get("hierarchies") else {
            panic!()
        };
        assert_eq!(hierarchies.len(), 1);
        let hierarchy = &hierarchies[0];
        let name = dimension.get("name").unwrap().as_str().unwrap();
        let mine = ours.hierarchy(name).unwrap();
        assert_eq!(
            hierarchy.get("name").unwrap().as_str(),
            Some(mine.name.as_str())
        );
        let Some(Json::Array(elements)) = hierarchy.get("elements") else {
            panic!()
        };
        let mut theirs_elements: Vec<(String, String)> = elements
            .iter()
            .map(|e| {
                (
                    e.get("Name").unwrap().py_str(),
                    e.get("Type").unwrap().py_str(),
                )
            })
            .collect();
        theirs_elements.sort();
        let mut my_elements: Vec<(String, String)> = mine
            .elements
            .iter()
            .map(|e| (e.name.clone(), e.element_type.clone()))
            .collect();
        my_elements.sort();
        assert_eq!(theirs_elements, my_elements, "{name}");
        let Some(Json::Array(edges)) = hierarchy.get("edges") else {
            panic!()
        };
        let mut theirs_edges: Vec<(String, String, String)> = edges
            .iter()
            .map(|e| {
                let weight = e.get("Weight").map_or("1".to_string(), Json::py_str);
                (
                    e.get("ParentName").unwrap().py_str(),
                    e.get("ComponentName").unwrap().py_str(),
                    d(&weight).to_fixed_string().unwrap(),
                )
            })
            .collect();
        theirs_edges.sort();
        let mut my_edges: Vec<(String, String, String)> = mine
            .edges
            .iter()
            .map(|e| {
                (
                    e.parent.clone(),
                    e.component.clone(),
                    e.weight.to_fixed_string().unwrap(),
                )
            })
            .collect();
        my_edges.sort();
        assert_eq!(theirs_edges.len(), my_edges.len(), "{name}");
        for ((p1, c1, w1), (p2, c2, w2)) in theirs_edges.iter().zip(&my_edges) {
            assert_eq!((p1, c1), (p2, c2), "{name}");
            assert_eq!(d(w1), d(w2), "{name}");
        }
    }
    let Some(Json::Array(cubes)) = theirs.get("cubes") else {
        panic!()
    };
    let mut theirs_cubes: Vec<(String, Vec<String>)> = cubes
        .iter()
        .map(|c| {
            let Some(Json::Array(dims)) = c.get("dimensions") else {
                panic!()
            };
            (
                c.get("name").unwrap().py_str(),
                dims.iter().map(Json::py_str).collect(),
            )
        })
        .collect();
    theirs_cubes.sort();
    let mut my_cubes: Vec<(String, Vec<String>)> = ours
        .cubes
        .iter()
        .map(|c| (c.name.clone(), c.dimensions.clone()))
        .collect();
    my_cubes.sort();
    assert_eq!(theirs_cubes, my_cubes);
    let Some(Json::Array(processes)) = theirs.get("processes") else {
        panic!()
    };
    let mut theirs_processes: Vec<(String, String)> = processes
        .iter()
        .map(|p| {
            (
                p.get("name").unwrap().py_str(),
                p.get("code_link").unwrap().py_str(),
            )
        })
        .collect();
    theirs_processes.sort();
    let mut my_processes: Vec<(String, String)> = ours
        .processes
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                p.script_source.as_ref().unwrap().name().to_string(),
            )
        })
        .collect();
    my_processes.sort();
    assert_eq!(theirs_processes, my_processes);
}

// ---------------------------------------------------------------- rules discipline

const VALUELESS: [&str; 2] = ["0", "1"];
const AMBIGUOUS: [&str; 2] = ["12", "100"];

fn structural(value: &Decimal) -> bool {
    VALUELESS.iter().chain(&AMBIGUOUS).any(|c| *value == d(c))
}

fn ruled_cubes(model: &Model) -> Vec<&str> {
    let mut names: Vec<&str> = model
        .cubes
        .iter()
        .filter(|c| c.rules.as_ref().is_some_and(|r| !r.rules.is_empty()))
        .map(|c| c.name.as_str())
        .collect();
    names.sort();
    names
}

fn guarded_sources(model: &Model) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = model
        .cubes
        .iter()
        .filter(|c| c.rules.is_some())
        .map(|c| {
            (
                format!("{}.rules", c.name),
                std::fs::read_to_string(c.rules_source.as_ref().unwrap().fs()).unwrap(),
            )
        })
        .collect();
    out.extend(
        model
            .processes
            .iter()
            .filter(|p| !p.script.is_empty())
            .map(|p| (format!("{}.ti", p.name), p.script.clone())),
    );
    out
}

fn rules_text(name: &str) -> String {
    std::fs::read_to_string(model_root().join("cubes").join(format!("{name}.rules"))).unwrap()
}

fn shipped_driver_values(model: &Model) -> Vec<(Vec<String>, Decimal)> {
    read_csv(
        &pp(&examples().join("drivers.csv")),
        model.cube("Drivers").unwrap(),
        model,
    )
    .unwrap()
}

fn shipped_driver_literal(model: &Model, coordinate: &[&str]) -> String {
    let values = shipped_driver_values(model);
    let (_, value) = values
        .iter()
        .find(|(c, _)| c == &coord(coordinate))
        .unwrap();
    value.to_fixed_string().unwrap()
}

/// `NUMERIC_LITERAL.finditer(text)`:
/// `(?<![\w.])(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?%?|(?<![\w.])\.\d+%?`.
fn numeric_literals(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let digit = |i: usize| i < c.len() && py::is_decimal(c[i]);
    let digits_from = |mut i: usize| {
        while digit(i) {
            i += 1;
        }
        i
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let free = i == 0 || !(py::is_word(c[i - 1]) || c[i - 1] == '.');
        let mut end = None;
        if free && digit(i) {
            // \d{1,3}(?:,\d{3})+ first, backtracking the leading run.
            let run = digits_from(i) - i;
            for lead in (1..=run.min(3)).rev() {
                let mut j = i + lead;
                let mut groups = 0;
                while j < c.len() && c[j] == ',' && (1..=3).all(|k| digit(j + k)) {
                    j += 4;
                    groups += 1;
                }
                if groups > 0 {
                    end = Some(j);
                    break;
                }
            }
            let mut j = end.unwrap_or_else(|| digits_from(i));
            if j < c.len() && c[j] == '.' && digit(j + 1) {
                j = digits_from(j + 1);
            }
            if j < c.len() && c[j] == '%' {
                j += 1;
            }
            end = Some(j);
        } else if free && c[i] == '.' && digit(i + 1) {
            let mut j = digits_from(i + 1);
            if j < c.len() && c[j] == '%' {
                j += 1;
            }
            end = Some(j);
        }
        match end {
            Some(j) => {
                found.push(c[i..j].iter().collect());
                i = j;
            }
            None => i += 1,
        }
    }
    found
}

fn figures_hard_coded_in(model: &Model, text: &str) -> Vec<String> {
    let guarded: Vec<Decimal> = shipped_driver_values(model)
        .into_iter()
        .map(|(_, v)| v)
        .filter(|v| !structural(v))
        .collect();
    let hundred = d("100");
    numeric_literals(text)
        .into_iter()
        .filter(|literal| {
            let written = d(&literal.trim_end_matches('%').replace(',', ""));
            let mut candidates = vec![if literal.ends_with('%') {
                written.div_(&hundred)
            } else {
                written.clone()
            }];
            if literal.contains('.') {
                candidates.push(written.div_(&hundred));
            }
            candidates.iter().any(|c| guarded.contains(c))
        })
        .collect()
}

#[test]
fn numeric_literals_follow_the_python_pattern() {
    // Each expectation is what Python's re.finditer gave for the same text.
    assert_eq!(
        numeric_literals("FY2025-26 1,234,567.89% 12,34 .0545 x.5 a1"),
        ["26", "1,234,567.89%", "12", "34", ".0545"]
    );
    assert_eq!(
        numeric_literals("1,2345 1234,567 ١٢,٣٤٥ 5.%"),
        ["1,234", "1234", "567", "١٢,٣٤٥", "5"]
    );
    assert_eq!(numeric_literals("..5 1.2.3"), ["1.2"]);
}

#[test]
fn every_calculated_cube_uses_skipcheck_and_source_cubes_feed() {
    let model = shipped();
    for name in ruled_cubes(&model) {
        assert!(
            model.cube(name).unwrap().rules.as_ref().unwrap().skipcheck,
            "{name}"
        );
    }
    for name in ["Workforce", "Revenue", "Capex"] {
        assert!(
            !model
                .cube(name)
                .unwrap()
                .rules
                .as_ref()
                .unwrap()
                .feeders
                .is_empty(),
            "{name}"
        );
    }
}

#[test]
fn every_month_of_depreciation_is_fed_by_name() {
    let model = shipped();
    let months = model
        .hierarchy("Period")
        .unwrap()
        .leaves_under("FY")
        .unwrap();
    let mut fed: Vec<&str> = Vec::new();
    for feeder in &model.cube("Capex").unwrap().rules.as_ref().unwrap().feeders {
        if feeder
            .target
            .elements
            .iter()
            .any(|e| e == "Depreciation Charge")
        {
            fed.extend(feeder.target.elements.iter().map(String::as_str));
        }
    }
    let unfed: Vec<&str> = months.into_iter().filter(|m| !fed.contains(m)).collect();
    assert!(
        unfed.is_empty(),
        "no feeder in Capex.rules names {}",
        unfed.join(", ")
    );
}

#[test]
fn the_reporting_cube_is_fed_from_its_source_cubes() {
    let model = shipped();
    let mut cross = Vec::new();
    for name in ["Workforce", "Revenue", "Capex"] {
        for feeder in &model.cube(name).unwrap().rules.as_ref().unwrap().feeders {
            if feeder.target_cube.as_deref() == Some("PnL") {
                let target = &feeder.target.elements;
                cross.push((name, target[target.len() - 2].clone()));
            }
        }
    }
    for expected in [
        ("Workforce", "Wages and Salaries"),
        ("Workforce", "Superannuation"),
        ("Workforce", "Payroll Tax"),
        ("Revenue", "Contract Revenue"),
        ("Revenue", "Plant Hire Revenue"),
        ("Revenue", "Fuel"),
        ("Capex", "Depreciation"),
    ] {
        assert!(
            cross
                .iter()
                .any(|(c, a)| *c == expected.0 && a == expected.1),
            "{expected:?}"
        );
    }
}

#[test]
fn no_statutory_rate_is_hard_coded_and_the_guard_bites() {
    let model = shipped();
    for (name, text) in guarded_sources(&model) {
        let found = figures_hard_coded_in(&model, &text);
        assert!(found.is_empty(), "{name} hard codes {}", found.join(", "));
    }
    let call = "DB('Drivers', !Year, !Version, 'Full Year', 'Maximum Contribution Base')";
    let text = rules_text("Workforce");
    assert!(text.contains(call));
    let literal = shipped_driver_literal(
        &model,
        &[
            "FY2025-26",
            "Actual",
            "Full Year",
            "Maximum Contribution Base",
        ],
    );
    let tampered = text.replacen(call, &literal, 1);
    assert!(figures_hard_coded_in(&model, &tampered).contains(&literal));

    let mut unguarded = Vec::new();
    for (coordinate, value) in shipped_driver_values(&model) {
        if structural(&value) {
            continue;
        }
        let literal = value.to_fixed_string().unwrap();
        let statement = format!("['Base Pay'] = N: ['Headcount'] * {literal};");
        if !figures_hard_coded_in(&model, &statement).contains(&literal) {
            unguarded.push(format!("{} = {literal}", coordinate.join(" ")));
        }
    }
    assert!(
        unguarded.is_empty(),
        "the guard misses {}",
        unguarded.join("; ")
    );

    let rate = shipped_driver_literal(
        &model,
        &["FY2026-27", "Budget", "Full Year", "Payroll Tax Rate"],
    );
    assert!(rate.starts_with("0."));
    assert!(
        !figures_hard_coded_in(
            &model,
            &format!("['Payroll Tax Cost'] = N: ['Base Pay'] * {};", &rate[1..])
        )
        .is_empty()
    );

    let collisions: Vec<String> = shipped_driver_values(&model)
        .iter()
        .filter(|(_, v)| AMBIGUOUS.iter().any(|a| *v == d(a)))
        .map(|(c, v)| format!("{} = {}", c.join(" "), v.to_fixed_string().unwrap()))
        .collect();
    assert!(collisions.is_empty(), "{collisions:?}");

    assert!(
        guarded_sources(&model)
            .iter()
            .any(|(n, _)| n.ends_with(".ti"))
    );
    let rate = shipped_driver_literal(&model, &["FY2026-27", "Budget", "Full Year", "SG Rate"]);
    assert!(!figures_hard_coded_in(&model, &format!("nRate = {rate};")).is_empty());
}

#[test]
fn the_shipped_rules_read_their_rates_from_drivers() {
    let text = rules_text("Workforce");
    assert!(
        text.contains("DB('Drivers'")
            && text.contains("'SG Rate'")
            && text.contains("'Maximum Contribution Base'")
    );
    assert!(text.contains("( ['Base Pay'] + ['Superannuation Cost'] )"));
    let text = rules_text("PnL");
    assert!(text.contains("'Payroll Tax Threshold'"));
    let specific = text.find("'CivilCo', 'Corporate', 'Payroll Tax'").unwrap();
    let general = text.find("['Payroll Tax', 'Amount']").unwrap();
    assert!(
        specific < general,
        "the specific statement must come before the general one"
    );
    let statement = &text[specific..general];
    assert!(statement.contains("'Days'") && !statement.contains("/ 12"));
    assert!(statement.contains("'Full Year', 'Days'") && !statement.contains("'FY', 'Days'"));
    assert!(!statement.contains('\\'));
}

#[test]
fn the_drivers_cube_calculates_nothing_and_feeds_the_threshold_credit() {
    let model = shipped();
    let rules = model.cube("Drivers").unwrap().rules.as_ref().unwrap();
    assert!(rules.rules.is_empty() && rules.skipcheck);
    assert_eq!(rules.feeders.len(), 1);
    let feeder = &rules.feeders[0];
    assert_eq!(feeder.target_cube.as_deref(), Some("PnL"));
    let mut source = feeder.area.elements.clone();
    source.sort();
    assert_eq!(source, ["Full Year", "Payroll Tax Threshold"]);
    assert_eq!(
        feeder.target.elements,
        [
            "!Year",
            "!Version",
            "FY",
            "CivilCo",
            "Corporate",
            "Payroll Tax",
            "Amount"
        ]
    );
}

#[test]
fn the_model_validates_with_no_errors_and_no_warnings() {
    let findings = validate_model(&shipped()).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
}
