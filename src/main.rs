//! # Contained
//!
//! Run a program in a Docker container.

use std::env;
use std::env::Args;
use anyhow::anyhow;
use contained::run;

fn main() -> Result<(), anyhow::Error> {
    let mut args: Args = env::args();
    args.next(); // ignore my own program name
    let program: String = args.next().ok_or(anyhow!("No program specified"))?;
    let arguments: Vec<String> = args.collect();

    let id = run(program.into(), &arguments)?;
    println!("{id}");
    Ok(())
}
