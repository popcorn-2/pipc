#![deny(warnings)]

use clap::{Arg, arg, command, value_parser};
use clap::parser::ValueSource;
use pipc::{OutputFormat, OutputTy, process_file};
use std::path::PathBuf;
use color_print::ceprintln;

fn main() {
    let cli = command!()
        .arg(
            arg!(-o --output <DIRECTORY> "directory to place generated output in")
                .default_value("output/")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            arg!(-l --language <LANGUAGE> "language to generate output in")
                .default_value("binary")
                .value_parser(value_parser!(OutputFormat)),
        )
        .arg(
            arg!(-f --format <FORMAT> "format of bindings")
                .default_value("client")
                .value_parser(value_parser!(OutputTy)),
        )
        .arg(Arg::new("files").num_args(1..).value_parser(value_parser!(PathBuf)).trailing_var_arg(true))
        .get_matches();

    let input_files = cli.get_many::<PathBuf>("files").unwrap().collect::<Vec<_>>();
    let language = cli.get_one::<OutputFormat>("language").unwrap();
    let format = cli.get_one::<OutputTy>("format").unwrap();
    let output = cli.get_one::<PathBuf>("output").unwrap();

    if *language == OutputFormat::Binary
        && cli.value_source("format").unwrap() != ValueSource::DefaultValue
    {
        ceprintln!(
            "<y><em>warning</em></y>: ignoring manual setting of client/server for binary format"
        );
    }

    let mut error_count = 0usize;
    for file in &input_files {
        match process_file(file, *language, *format, output) {
            Err(e) => {
                eprintln!("{e}");
                error_count += 1;
            }
            Ok(_) => {}
        }
    }
    if error_count != 0 {
        ceprintln!("<r><em>error</em></r>: {error_count} errors generated")
    }
}
