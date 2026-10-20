//! Time the evaluator on fabricated workloads and the shipped model.
//!
//! The `planning-cycle` workload builds a small fabricated model, changes an
//! input, recalculates, reads the affected result and evaluates an extended
//! model. It checks independent arithmetic before accepting a timing sample,
//! so a fast incorrect result fails the run. Every other workload records a
//! SHA-256 fingerprint of its result, which must be the same on every sample.
//!
//! ```text
//! cargo bench --bench evaluate -- --repeats 5 --output /approved/path/benchmark.json
//! ```
//!
//! The report measures this engine only. Comparing two revisions means running
//! the benchmark at each and comparing the reports; the fingerprints show
//! whether the two computed the same results.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use pacioliscube::data::load_data;
use pacioliscube::decimal::Decimal;
use pacioliscube::evaluate::{CellStore, consolidate_many, evaluate, explain};
use pacioliscube::model::{Model, load_model};
use pacioliscube::pypath::PyPath;
use sha2::{Digest, Sha256};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A model tree with the given dimensions (name, leaf elements) and one cube.
fn write_tree(root: &Path, cube: &str, dimensions: &[(String, Vec<String>)], rules: &str) {
    let mut links = Vec::new();
    for (name, elements) in dimensions {
        let listed: Vec<String> = elements
            .iter()
            .map(|e| format!(r#"{{"Name": "{e}"}}"#))
            .collect();
        write(
            &root.join(format!("dimensions/{name}.json")),
            &format!(r#"{{"Name": "{name}", "Hierarchies@Code.links": ["{name}.h/{name}.json"]}}"#),
        );
        write(
            &root.join(format!("dimensions/{name}.h/{name}.json")),
            &format!(
                r#"{{"Name": "{name}", "Elements": [{}], "Edges": []}}"#,
                listed.join(", ")
            ),
        );
        links.push(format!(r#""dimensions/{name}.json""#));
    }
    let refs: Vec<String> = dimensions
        .iter()
        .map(|(n, _)| format!(r#"{{"@id": "Dimensions('{n}')"}}"#))
        .collect();
    write(
        &root.join(format!("cubes/{cube}.json")),
        &format!(
            r#"{{"Name": "{cube}", "Dimensions": [{}], "Rules@Code.link": "{cube}.rules"}}"#,
            refs.join(", ")
        ),
    );
    write(&root.join(format!("cubes/{cube}.rules")), rules);
    write(
        &root.join("tm1project.json"),
        &format!(
            r#"{{"Name": "{cube}", "Objects": {{"Dimensions": [{}], "Cubes": ["cubes/{cube}.json"]}}}}"#,
            links.join(", ")
        ),
    );
}

fn load(root: &Path) -> Model {
    load_model(&PyPath::new(&root.to_string_lossy())).unwrap()
}

fn workload(root: &Path, entities: usize, rules: usize) -> (Model, CellStore) {
    let names: Vec<String> = (0..entities).map(|i| format!("Entity{i}")).collect();
    let mut measures = vec!["Input".to_string()];
    measures.extend((0..rules).map(|i| format!("Output{i}")));
    let text: Vec<String> = (0..rules)
        .map(|i| {
            format!("['Output{i}'] = N: IF(['Input'] > 0, ['Input'] * 2 + {i}, ['Input'] / 0);")
        })
        .collect();
    write_tree(
        root,
        "Synthetic",
        &[
            ("Entity".into(), names.clone()),
            ("Measure".into(), measures),
        ],
        &text.join("\n"),
    );
    let model = load(root);
    let mut store = CellStore::new();
    for (index, entity) in names.iter().enumerate() {
        let value = Decimal::from_int(index as i64 + 1)
            .div(&Decimal::from_int(10))
            .unwrap();
        store.set("Synthetic", &[entity.clone(), "Input".into()], value);
    }
    (model, store)
}

fn d(text: &str) -> Decimal {
    Decimal::parse(text).unwrap()
}

/// Construct, change inputs, recalculate and query a changed synthetic model.
fn planning_cycle(scratch: &Path) -> Vec<Decimal> {
    let cell = |output: &str| vec!["Entity0".to_string(), output.to_string()];
    let (model, inputs) = workload(&scratch.join("cycle-a"), 3, 2);
    let initial = evaluate(&model, &inputs)
        .unwrap()
        .get("Synthetic", &cell("Output0"));
    let mut changed = inputs.clone();
    changed.set("Synthetic", &cell("Input"), d("2"));
    let recalculated = evaluate(&model, &changed)
        .unwrap()
        .get("Synthetic", &cell("Output0"));
    let (wider, wider_inputs) = workload(&scratch.join("cycle-b"), 3, 3);
    let extended = evaluate(&wider, &wider_inputs)
        .unwrap()
        .get("Synthetic", &cell("Output2"));
    let result = vec![initial, recalculated, extended];
    // 0.1 * 2, then 2 * 2, then 0.1 * 2 + 2: worked out by hand.
    let expected = vec![d("0.2"), d("4"), d("2.2")];
    assert!(
        result == expected,
        "planning-cycle arithmetic differs from the independent expectation"
    );
    result
}

fn fingerprint(values: &[String]) -> String {
    let mut hash = Sha256::new();
    for value in values {
        hash.update(value.as_bytes());
        hash.update(b"\n");
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn store_text(store: &CellStore) -> Vec<String> {
    store
        .items()
        .map(|(c, x, v)| format!("{c}{x:?}={}", v.to_sci_string()))
        .collect()
}

struct Report {
    workloads: Vec<(String, String, Vec<f64>)>,
}

impl Report {
    fn time(&mut self, name: &str, repeats: usize, mut operation: impl FnMut() -> Vec<String>) {
        let mut seconds = Vec::new();
        let mut signature: Option<String> = None;
        for _ in 0..repeats {
            let started = Instant::now();
            let result = operation();
            seconds.push(started.elapsed().as_secs_f64());
            let this = fingerprint(&result);
            if let Some(previous) = &signature {
                assert_eq!(previous, &this, "output differs for {name}");
            }
            signature = Some(this);
            drop(result);
        }
        let mut sorted = seconds.clone();
        sorted.sort_by(f64::total_cmp);
        println!("{name}: median {:.6} s", sorted[sorted.len() / 2]);
        self.workloads
            .push((name.to_string(), signature.unwrap(), seconds));
    }

    fn json(&self) -> String {
        let mut out = String::from("{\n  \"engine\": \"pacioliscube ");
        out.push_str(env!("CARGO_PKG_VERSION"));
        let _ = write!(
            out,
            "\",\n  \"platform\": \"{} {}\",\n  \"workloads\": {{",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        for (i, (name, sha, seconds)) in self.workloads.iter().enumerate() {
            let mut sorted = seconds.clone();
            sorted.sort_by(f64::total_cmp);
            let list: Vec<String> = seconds.iter().map(|s| format!("{s:.6}")).collect();
            let _ = write!(
                out,
                "{}\n    \"{name}\": {{\"sha256\": \"{sha}\", \"seconds\": [{}], \"median_seconds\": {:.6}, \"min_seconds\": {:.6}, \"max_seconds\": {:.6}}}",
                if i > 0 { "," } else { "" },
                list.join(", "),
                sorted[sorted.len() / 2],
                sorted[0],
                sorted[sorted.len() - 1]
            );
        }
        out.push_str("\n  }\n}\n");
        out
    }
}

fn main() {
    let mut repeats = 5usize;
    let mut output = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repeats" => {
                repeats = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|n| *n > 0)
                    .expect("--repeats must be positive")
            }
            "--output" => output = args.next(),
            _ => {}
        }
    }
    let scratch = tempfile::tempdir().unwrap();
    let mut report = Report {
        workloads: Vec::new(),
    };
    report.time("planning-cycle", repeats, || {
        planning_cycle(scratch.path())
            .iter()
            .map(Decimal::to_sci_string)
            .collect()
    });

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shipped = load(&repo.join("model"));
    let shipped_store = load_data(
        &shipped,
        &PyPath::new(&repo.join("examples").to_string_lossy()),
    )
    .unwrap();
    let mut cases = vec![("repository-model".to_string(), shipped, shipped_store)];
    for (name, entities, rules) in [("small", 100, 4), ("medium", 500, 16), ("large", 1000, 64)] {
        let (model, store) = workload(&scratch.path().join(name), entities, rules);
        cases.push((name.to_string(), model, store));
    }
    for (name, model, store) in &cases {
        report.time(name, repeats, || {
            store_text(&evaluate(model, store).unwrap())
        });
        if name == "repository-model" || name == "large" {
            let calculated = evaluate(model, store).unwrap();
            let names: Vec<(String, String)> = model
                .cubes
                .iter()
                .map(|c| (pacioliscube::py::casefold(&c.name), c.name.clone()))
                .collect();
            let cells: Vec<(String, Vec<String>)> = calculated
                .items()
                .map(|(cube, coordinate, _)| {
                    (
                        names.iter().find(|(k, _)| k == cube).unwrap().1.clone(),
                        coordinate.clone(),
                    )
                })
                .collect();
            let last = |n: usize| cells[cells.len().saturating_sub(n)..].to_vec();
            let batch = last(1000);
            report.time(&format!("{name}-batch"), repeats, || {
                let mut values = Vec::new();
                consolidate_many(model, store, &batch, |_, v| values.push(v.to_sci_string()))
                    .unwrap();
                values
            });
            let explained = last(64);
            report.time(&format!("{name}-explain"), repeats, || {
                let evidence = explain(model, store, &explained).unwrap();
                evidence
                    .cells
                    .iter()
                    .map(|(label, e)| {
                        format!("{label}={:?}", e.value.as_ref().map(Decimal::to_sci_string))
                    })
                    .collect()
            });
        }
    }

    // 64 different sets of selected dimensions with an initial miss, then 1,000 early matches.
    let root = scratch.path().join("shapes");
    let mut dimensions = vec![(
        "Entity".to_string(),
        (0..1000).map(|i| format!("E{i}")).collect::<Vec<_>>(),
    )];
    for i in 0..7 {
        dimensions.push((format!("D{i}"), vec![format!("D{i}a"), format!("D{i}b")]));
    }
    let mut areas: Vec<Vec<String>> = vec![vec!["D0a".into()]];
    for length in 1..=6 {
        let mut subset: Vec<usize> = (1..=length).collect();
        loop {
            areas.push(subset.iter().map(|i| format!("D{i}a")).collect());
            // Next combination of `length` items from 1..=6, in lexicographic order.
            let mut k = length;
            while k > 0 && subset[k - 1] == 6 - length + k {
                k -= 1;
            }
            if k == 0 {
                break;
            }
            subset[k - 1] += 1;
            for j in k..length {
                subset[j] = subset[j - 1] + 1;
            }
        }
    }
    let text: Vec<String> = areas
        .iter()
        .enumerate()
        .map(|(i, area)| {
            let names: Vec<String> = area.iter().map(|e| format!("'{e}'")).collect();
            format!("[{}] = N: {};", names.join(", "), i + 1)
        })
        .collect();
    write_tree(&root, "Shapes", &dimensions, &text.join("\n"));
    let model = load(&root);
    let mut cells = vec![("Shapes".to_string(), {
        let mut c = vec!["E0".to_string()];
        c.extend((0..7).map(|i| format!("D{i}b")));
        c
    })];
    for i in 0..1000 {
        let mut c = vec![format!("E{i}"), "D0a".to_string()];
        c.extend((1..7).map(|j| format!("D{j}b")));
        cells.push(("Shapes".to_string(), c));
    }
    let store = CellStore::new();
    report.time("selector-shapes-early-match", repeats, || {
        let mut values = Vec::new();
        consolidate_many(&model, &store, &cells, |_, v| {
            values.push(v.to_sci_string())
        })
        .unwrap();
        values
    });

    if let Some(path) = output {
        std::fs::write(path, report.json()).unwrap();
    }
}
