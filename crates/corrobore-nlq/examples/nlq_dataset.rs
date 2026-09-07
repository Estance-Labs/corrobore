// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT
//! Generate the pilot corpus, split it, evaluate the template compiler and
//! print the report as JSON.
//!
//! Usage: `cargo run -p corrobore-nlq --example nlq_dataset -- [seed] [corpus|splits|report]`
//! The default seed is 42 and the default output is the evaluation report.

use corrobore_nlq::{
    TemplateCompiler,
    dataset::{generate, split},
    evaluation::evaluate,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let seed: u64 = arguments.next().map_or(Ok(42), |value| value.parse())?;
    let output = arguments.next().unwrap_or_else(|| "report".to_owned());
    let corpus = generate(seed);
    match output.as_str() {
        "corpus" => println!("{}", serde_json::to_string_pretty(&corpus)?),
        "splits" => println!("{}", serde_json::to_string_pretty(&split(&corpus, seed))?),
        "report" => {
            let report = evaluate(&TemplateCompiler, &corpus.examples);
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        other => {
            return Err(format!("unknown output `{other}`; use corpus, splits or report").into());
        }
    }
    Ok(())
}
