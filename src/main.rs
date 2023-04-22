//! # Contained
//!
//! Run a program in a Docker container.

use std::env;
use std::env::Args;
use std::error::Error;
use contained::create_container;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args: Args = env::args();
    args.next(); // ignore my own program name
    let program: String = args.next().ok_or("No program specified")?;
    let arguments: Vec<String> = args.collect();

    let (status_code, value) = create_container(program, &arguments)?;
    println!("{status_code}");
    println!("{value}");

    Ok(())
}
