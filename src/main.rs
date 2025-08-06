use std::error::Error;
use parser::parse_program;

mod parser;

fn main() -> Result<(), Box<dyn Error>> {
    let input = std::fs::read_to_string("test.wp")?;
    match parse_program(&input) {
        Ok((leftover_input, output)) => {
            println!("Leftover input: {}", leftover_input);
            println!("Output: \n{}", output);
            Ok(())
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    }
}