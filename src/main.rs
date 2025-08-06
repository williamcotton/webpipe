use nom::IResult;
use std::error::Error;
pub use nom::{branch::alt, bytes::complete::{tag, take_till1}, character::complete::multispace0, Parser, multi::many0 };
use std::fmt::Display;

struct Route {
    method: String,
    path: String,
    pipeline: Pipeline,
}

impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.method, self.path, self.pipeline)
    }
}

struct Pipeline {
    steps: Vec<PipelineStep>
}

impl Display for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let steps_str: Vec<String> = self.steps.iter().map(|s| s.to_string()).collect();
        write!(f, "\n  |> {}", steps_str.join("\n  |> "))
    }
}

struct PipelineStep {
    name: String,
    config: String
}

impl Display for PipelineStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: `{}`", self.name, self.config)
    }
}

struct Program {
    routes: Vec<Route>,
}

impl Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let routes_str: Vec<String> = self.routes.iter().map(|r| r.to_string()).collect();
        write!(f, "{}", routes_str.join("\n"))
    }
}

fn parse_method(input: &str) -> IResult<&str, &str> {
    alt((
        tag("GET"),
        tag("POST"),
        tag("PUT"),
        tag("DELETE"),
    )).parse(input)
}

fn parse_pipeline_step(input: &str) -> IResult<&str, PipelineStep> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = take_till1(|c| c == ':')(input)?;
    let (input, _) = tag(":")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("`")(input)?;
    let (input, config) = take_till1(|c| c == '`')(input)?;
    let (input, _) = tag("`")(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, PipelineStep { name: name.to_string(), config: config.to_string() }))
}

fn parse_pipeline(input: &str) -> IResult<&str, Pipeline> {
    let (input, steps) = many0(parse_pipeline_step).parse(input)?;
    Ok((input, Pipeline { steps }))
}

fn parse_route(input: &str) -> IResult<&str, Route> {
    let (input, method) = parse_method(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = take_till1(|c| c == ' ' || c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Route { method: method.to_string(), path: path.to_string(), pipeline }))
}

fn parse_program(input: &str) -> IResult<&str, Program> {
    let (input, routes) = many0(parse_route).parse(input)?;
    Ok((input, Program { routes }))
}

const TEST_INPUT: &str = "GET /test\n\
        |> test: `test`\n\
        |> test2: `test2\ntest2`\n\
    GET /test5 |> test2: `test2`\n\
    GET /test6 |> test3: `test3`";

fn main() -> Result<(), Box<dyn Error>> {
    let (leftover_input, output) = parse_program(TEST_INPUT)?;
    println!("Leftover input: {}", leftover_input);
    println!("Output: {}", output);
    Ok(())
}