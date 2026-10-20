//! Read long format CSV input into cube coordinates, a file or a directory at a time.
//!
//! The engine reads the same synthetic CSV files that the TurboIntegrator
//! processes read on a real server. A row carries one leading column per cube
//! dimension, in the cube's dimension order, then a value column. Every row is
//! checked before it is used, and every error names the file and the row.

use std::collections::HashMap;

use crate::csvread::{CsvError, Reader};
use crate::decimal::Decimal;
use crate::error::{EXIT_USAGE, Error, Result, Uncaught};
use crate::evaluate::CellStore;
use crate::model::{Cube, Model};
use crate::py::{self, OsStyle};
use crate::pypath::{self, PyPath};

/// The stem of a data file names the cube it feeds. The shipped example files are
/// listed rather than derived, because pnl-direct is not spelt like the cube it
/// loads. A stem that matches a cube name is taken as well.
pub const CUBE_BY_STEM: [(&str, &str); 5] = [
    ("drivers", "Drivers"),
    ("workforce", "Workforce"),
    ("revenue", "Revenue"),
    ("capex", "Capex"),
    ("pnl-direct", "PnL"),
];

fn csv_failure(error: CsvError, path: &PyPath) -> Error {
    match error {
        CsvError::Decode(e) => Error::Decode(e.to_string()),
        CsvError::Io(e) => Error::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)),
        CsvError::Csv(message) => Error::Uncaught(Uncaught::new("_csv.Error", message)),
    }
}

/// Every data row of a long format CSV as a coordinate and a value, handed to
/// `emit` as it is read, which is when Python's generator yielded it.
pub fn load_csv(
    path: &PyPath,
    cube: &Cube,
    model: &Model,
    mut emit: impl FnMut(Vec<String>, Decimal),
) -> Result<usize> {
    if !path.is_file() {
        return Err(Error::model(format!("{path}: file not found")));
    }
    let file = std::fs::File::open(path.fs())
        .map_err(|e| Error::Os(py::oserror_str(&e, &path.display(), OsStyle::Errno)))?;
    let mut reader = Reader::new(std::io::BufReader::new(file));
    let width = cube.dimensions.len() + 1;
    let Some(header) = reader.next_record().map_err(|e| csv_failure(e, path))? else {
        return Err(Error::model(format!(
            "{path}: the file is empty, expected a header row"
        )));
    };
    if header.len() != width {
        return Err(Error::model(format!(
            "{path} row 1: the header has {} columns but cube {} needs {width}, being {} then a value",
            header.len(),
            py::repr_str(&cube.name),
            cube.dimensions.join(", ")
        )));
    }
    let mut seen: HashMap<Vec<String>, (Decimal, usize)> = HashMap::new();
    let mut written = 0;
    let mut number = 1;
    while let Some(row) = reader.next_record().map_err(|e| csv_failure(e, path))? {
        number += 1;
        if row.iter().all(|field| py::strip(field).is_empty()) {
            continue;
        }
        if row.len() != width {
            return Err(Error::model(format!(
                "{path} row {number}: {} columns, expected {width}",
                row.len()
            )));
        }
        let mut coordinate = Vec::with_capacity(width - 1);
        for (position, field) in row[..width - 1].iter().enumerate() {
            let dimension = &cube.dimensions[position];
            let element = py::strip(field);
            let resolved = (|| -> Result<String> {
                let hierarchy = model.hierarchy(dimension)?;
                let canonical = hierarchy.resolve(element)?;
                if !hierarchy.is_leaf(canonical)? {
                    return Err(Error::model(format!(
                        "{dimension}: element {} is consolidated and cannot hold input",
                        py::repr_str(canonical)
                    )));
                }
                Ok(canonical.to_string())
            })();
            match resolved {
                Ok(canonical) => coordinate.push(canonical),
                Err(error) if error.is_model_error() => {
                    return Err(Error::model(format!(
                        "{path} row {number}: {}",
                        error.message()
                    )));
                }
                Err(error) => return Err(error),
            }
        }
        let text = py::strip(&row[width - 1]);
        let value = Decimal::parse(text).map_err(|_| {
            Error::model(format!(
                "{path} row {number}: value {} is not a number",
                py::repr_str(text)
            ))
        })?;
        if !value.is_finite() {
            return Err(Error::model(format!(
                "{path} row {number}: value {} is not a finite number",
                py::repr_str(text)
            )));
        }
        if let Some((previous, row_number)) = seen.get(&coordinate) {
            if *previous != value {
                return Err(Error::model(format!(
                    "{path} row {number}: conflicting value for {} at {}; first supplied on row {row_number}",
                    py::repr_str(&cube.name),
                    py::repr_list(&coordinate)
                )));
            }
            continue;
        }
        emit(coordinate.clone(), value.clone());
        seen.insert(coordinate, (value, number));
        written += 1;
    }
    Ok(written)
}

/// `list(load_csv(path, cube, model))`.
pub fn read_csv(path: &PyPath, cube: &Cube, model: &Model) -> Result<Vec<(Vec<String>, Decimal)>> {
    let mut rows = Vec::new();
    load_csv(path, cube, model, |c, v| rows.push((c, v)))?;
    Ok(rows)
}

/// Load one CSV into a cell store and return how many cells it wrote.
pub fn load_into_store(
    model: &Model,
    cube_name: &str,
    path: &PyPath,
    store: &mut CellStore,
) -> Result<usize> {
    let Some(cube) = model.cube(cube_name) else {
        return Err(Error::model(format!(
            "no cube named {} in model {}",
            py::repr_str(cube_name),
            model.name_repr()
        )));
    };
    load_csv(path, cube, model, |coordinate, value| {
        store.set(&cube.name, &coordinate, value)
    })
}

/// Which cube a data file loads into, by its stem.
pub fn cube_for_file<'a>(model: &'a Model, path: &PyPath) -> Result<&'a Cube> {
    let stem = py::casefold(path.stem());
    if let Some((_, named)) = CUBE_BY_STEM.iter().find(|(s, _)| *s == stem)
        && let Some(cube) = model.cube(named)
    {
        return Ok(cube);
    }
    if let Some(cube) = model.cubes.iter().find(|c| py::casefold(&c.name) == stem) {
        return Ok(cube);
    }
    Err(Error::cli(
        EXIT_USAGE,
        format!(
            "{path}: no cube in model {} matches the file name {}",
            model.name_repr(),
            py::repr_str(path.stem())
        ),
    ))
}

/// `directory.iterdir()`, filtered to CSV files and sorted as `sorted()` did.
/// A CSV whose name is not Unicode text is refused rather than skipped.
fn csv_files(directory: &PyPath) -> Result<Vec<PyPath>> {
    let unreadable = |e: std::io::Error| {
        Error::cli(
            EXIT_USAGE,
            format!(
                "{directory}: the data directory could not be read, {}",
                py::oserror_str(&e, &directory.display(), OsStyle::Native)
            ),
        )
    };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory.fs()).map_err(unreadable)? {
        let entry = entry.map_err(unreadable)?;
        let raw = entry.file_name();
        let suffix = py::casefold(PyPath::new(&raw.to_string_lossy()).suffix());
        if suffix != ".csv" || !std::fs::metadata(entry.path()).is_ok_and(|m| m.is_file()) {
            continue;
        }
        files.push(directory.join(&pypath::fs_text(&raw)?));
    }
    files.sort();
    Ok(files)
}

/// Load every CSV in a directory into one store, one file per cube.
pub fn load_data(model: &Model, directory: &PyPath) -> Result<CellStore> {
    let mut store = CellStore::new();
    let boundary = directory.resolve()?;
    let files = csv_files(directory)?;
    if files.is_empty() {
        return Err(Error::cli(
            EXIT_USAGE,
            format!("{directory}: there are no CSV files there"),
        ));
    }

    // Every file is matched to its cube before any of them is read: two files
    // feeding one cube would leave the cells they share holding whichever sorted
    // second, so both are refused.
    let mut feeding: Vec<(String, Vec<PyPath>)> = Vec::new();
    for path in files {
        if !path.resolve()?.is_relative_to(&boundary) {
            return Err(Error::cli(
                EXIT_USAGE,
                format!("{path}: this file resolves outside the data directory"),
            ));
        }
        let cube = cube_for_file(model, &path)?.name.clone();
        match feeding.iter_mut().find(|(c, _)| *c == cube) {
            Some((_, paths)) => paths.push(path),
            None => feeding.push((cube, vec![path])),
        }
    }
    for (cube_name, paths) in &feeding {
        if paths.len() > 1 {
            let names: Vec<String> = paths.iter().map(PyPath::display).collect();
            return Err(Error::cli(
                EXIT_USAGE,
                format!(
                    "cube {} is fed by more than one file: {}. Leave the one that belongs to \
                     this run in the data directory and move the rest out",
                    py::repr_str(cube_name),
                    names.join(", ")
                ),
            ));
        }
    }

    for (cube_name, paths) in &feeding {
        let path = &paths[0];
        match load_into_store(model, cube_name, path, &mut store) {
            Ok(_) => {}
            Err(error) if error.is_model_error() => {
                return Err(Error::cli(EXIT_USAGE, error.message()));
            }
            Err(Error::Decode(message)) => {
                return Err(Error::cli(
                    EXIT_USAGE,
                    format!(
                        "{path}: this file is not UTF-8 text ({message}). A spreadsheet export \
                         saves as UTF-8 from its own save dialogue"
                    ),
                ));
            }
            Err(Error::Os(message)) => {
                return Err(Error::cli(
                    EXIT_USAGE,
                    format!("{path}: this file could not be read, {message}"),
                ));
            }
            Err(other) => return Err(other),
        }
    }
    Ok(store)
}
