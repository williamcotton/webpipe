use nom::IResult;
use std::error::Error;
pub use nom::{branch::alt, bytes::complete::{tag, take_till1, take_till}, character::complete::multispace0, Parser, multi::many0 };
use std::fmt::Display;

struct Route {
    method: String,
    path: String,
    pipeline: Pipeline,
}

impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}\n", self.method, self.path, self.pipeline)
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

struct Describe {
    name: String,
    it: Vec<It>,
}

impl Display for Describe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let it_str: Vec<String> = self.it.iter().map(|i| i.to_string()).collect();
        write!(f, "describe \"{}\"\n  {}", self.name, it_str.join("\n  "))
    }
}


struct It {
    name: String,
    when: String,
    then: String,
    output: String,
}

impl Display for It {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "it \"{}\"\n    {}\n    {}\n    {}", self.name, self.when, self.then, self.output)
    }
}

struct Program {
    routes: Vec<Route>,
    describe: Vec<Describe>,
}

impl Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let routes_str: Vec<String> = self.routes.iter().map(|r| r.to_string()).collect();
        let describe_str: Vec<String> = self.describe.iter().map(|d| d.to_string()).collect();
        write!(f, "{}", routes_str.join("\n"))?;
        write!(f, "\n{}", describe_str.join("\n"))
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
    let (input, config) = take_till(|c| c == '`')(input)?;
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

fn parse_it(input: &str) -> IResult<&str, It> {
    let (input, _) = tag("it")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("\"")(input)?;
    let (input, name) = take_till1(|c| c == '\"')(input)?;
    let (input, _) = tag("\"")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, when) = take_till1(|c| c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, then) = take_till1(|c| c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, output) = take_till1(|c| c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, It { name: name.to_string(), when: when.to_string(), then: then.to_string(), output: output.to_string() }))
}

fn parse_describe(input: &str) -> IResult<&str, Describe> {
    let (input, _) = tag("describe")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("\"")(input)?;
    let (input, name) = take_till1(|c| c == '\"')(input)?;
    let (input, _) = tag("\"")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, it) = many0(parse_it).parse(input)?;
    Ok((input, Describe { name: name.to_string(), it }))
}


fn parse_program(input: &str) -> IResult<&str, Program> {
    let (input, routes) = many0(parse_route).parse(input)?;
    let (input, describe) = many0(parse_describe).parse(input)?;
    Ok((input, Program { routes, describe }))
}

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