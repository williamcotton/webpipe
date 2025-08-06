use nom::IResult;
use std::error::Error;
pub use nom::{bytes::complete::tag, character::complete::multispace0};
use std::fmt::Display;

struct Route {
    method: String,
    path: String,
}

impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.method, self.path)
    }
}

struct Program {
    routes: Vec<Route>,
}


fn parse_route(input: &str) -> IResult<&str, Route> {
    let (input, method) = tag("GET")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = tag("/")(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Route { method: method.to_string(), path: path.to_string() }))
}

fn main() -> Result<(), Box<dyn Error>> {
    let (leftover_input, output) = parse_route("GET /test")?;
    println!("Leftover input: {}", leftover_input);
    println!("Output: {}", output);
    Ok(())
}