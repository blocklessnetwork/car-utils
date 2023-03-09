use std::process::ExitCode;

use clap::{Arg, ArgMatches, Command};

mod archive;
mod extract;
mod error;
mod ls;
use archive::archive_local_fs;
use extract::extract_car;

fn clap_matches() -> ArgMatches {
    Command::default()
        .arg_required_else_help(true)
        .subcommand(
            Command::new("ar")
                .about("archive local file system to a car file")
                .arg(Arg::new("car")
                    .short('c')
                    .required(true)
                    .help("the car file for archive.")
                )
                .arg(Arg::new("source")
                    .short('s')
                    .required(true)
                    .help("the source directory to archived")
                )
        )
        .subcommand(
            Command::new("ls")
                .about("list the car files")
                .arg(Arg::new("car")
                    .short('c')
                    .required(true)
                    .help("the car file for list.")
                )
        )
        .subcommand(
            Command::new("ex")
                .about("extract the car files")
                .arg(Arg::new("car")
                    .short('c')
                    .required(true)
                    .help("the car file for extract")
                )
                .arg(Arg::new("target")
                    .short('t')
                    .required(false)
                    .help("the target directory to extract")
                )
        )
        .get_matches()
}

fn main() -> ExitCode  {
    let command = clap_matches();
    let result = match command.subcommand() {
        Some(("ar", subcommad)) => {
            let car = subcommad.get_one::<String>("car").unwrap();
            let source = subcommad.get_one::<String>("source").unwrap();
            archive_local_fs(source, car)
        }
        Some(("ls", subcommad)) => {
            let car = subcommad.get_one::<String>("car").unwrap();
            ls::list_car_file(car)
        }
        Some(("ex", subcommad)) => {
            let car = subcommad.get_one::<String>("car").unwrap();
            let target = subcommad.get_one::<String>("target");
            extract_car(car, target)
        }
        _ => unreachable!("should not be reached."),
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e.err);
            e.into()
        },
    }
}
