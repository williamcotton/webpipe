use nom::IResult;
pub use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1, take_until},
    character::complete::{multispace0, alphanumeric1, char, digit1},
    sequence::{delimited, preceded},
    combinator::{opt, recognize, map, map_res},
    multi::{many0},
    Parser
};
use std::fmt::Display;

#[derive(Debug, Clone)]
pub struct Program {
    pub configs: Vec<Config>,
    pub pipelines: Vec<NamedPipeline>,
    pub variables: Vec<Variable>,
    pub routes: Vec<Route>,
    pub describes: Vec<Describe>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub properties: Vec<ConfigProperty>,
}

#[derive(Debug, Clone)]
pub struct ConfigProperty {
    pub key: String,
    pub value: ConfigValue,
}

#[derive(Debug, Clone)]
pub enum ConfigValue {
    String(String),
    EnvVar { var: String, default: Option<String> },
    Boolean(bool),
    Number(i64),
}

#[derive(Debug, Clone)]
pub struct NamedPipeline {
    pub name: String,
    pub pipeline: Pipeline,
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub var_type: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub method: String,
    pub path: String,
    pub pipeline: PipelineRef,
}

#[derive(Debug, Clone)]
pub enum PipelineRef {
    Inline(Pipeline),
    Named(String),
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub steps: Vec<PipelineStep>,
}

#[derive(Debug, Clone)]
pub struct PipelineStep {
    pub name: String,
    pub config: String,
}

#[derive(Debug, Clone)]
pub struct Describe {
    pub name: String,
    pub mocks: Vec<Mock>,
    pub tests: Vec<It>,
}

#[derive(Debug, Clone)]
pub struct Mock {
    pub target: String,
    pub return_value: String,
}

#[derive(Debug, Clone)]
pub struct It {
    pub name: String,
    pub mocks: Vec<Mock>,
    pub when: When,
    pub input: Option<String>,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone)]
pub enum When {
    CallingRoute { method: String, path: String },
    ExecutingPipeline { name: String },
    ExecutingVariable { var_type: String, name: String },
}

#[derive(Debug, Clone)]
pub struct Condition {
    pub condition_type: ConditionType,
    pub field: String,
    pub comparison: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub enum ConditionType {
    Then,
    And,
}

impl Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for config in &self.configs {
            writeln!(f, "{}", config)?;
        }
        for pipeline in &self.pipelines {
            writeln!(f, "{}", pipeline)?;
        }
        for variable in &self.variables {
            writeln!(f, "{}", variable)?;
        }
        for route in &self.routes {
            writeln!(f, "{}", route)?;
        }
        for describe in &self.describes {
            writeln!(f, "{}", describe)?;
        }
        Ok(())
    }
}

impl Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "config {} {{", self.name)?;
        for prop in &self.properties {
            writeln!(f, "  {}", prop)?;
        }
        write!(f, "}}")
    }
}

impl Display for ConfigProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.value)
    }
}

impl Display for ConfigValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigValue::String(s) => write!(f, "\"{}\"", s),
            ConfigValue::EnvVar { var, default: Some(def) } => write!(f, "${} || \"{}\"", var, def),
            ConfigValue::EnvVar { var, default: None } => write!(f, "${}", var),
            ConfigValue::Boolean(b) => write!(f, "{}", b),
            ConfigValue::Number(n) => write!(f, "{}", n),
        }
    }
}

impl Display for NamedPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pipeline {} = {}", self.name, self.pipeline)
    }
}

impl Display for Variable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} = `{}`", self.var_type, self.name, self.value)
    }
}

impl Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}\n{}", self.method, self.path, self.pipeline)
    }
}

impl Display for PipelineRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineRef::Inline(pipeline) => write!(f, "{}", pipeline),
            PipelineRef::Named(name) => write!(f, "  |> pipeline: {}", name),
        }
    }
}

impl Display for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let steps_str: Vec<String> = self.steps.iter().map(|s| s.to_string()).collect();
        write!(f, "{}", steps_str.join("\n"))
    }
}

impl Display for PipelineStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "  |> {}: `{}`", self.name, self.config)
    }
}

impl Display for Describe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "describe \"{}\"", self.name)?;
        for mock in &self.mocks {
            writeln!(f, "  {}", mock)?;
        }
        for test in &self.tests {
            writeln!(f, "  {}", test)?;
        }
        Ok(())
    }
}

impl Display for Mock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "with mock {} returning `{}`", self.target, self.return_value)
    }
}

impl Display for It {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "it \"{}\"", self.name)?;
        for mock in &self.mocks {
            writeln!(f, "    {}", mock)?;
        }
        writeln!(f, "    when {}", self.when)?;
        if let Some(input) = &self.input {
            writeln!(f, "    with input `{}`", input)?;
        }
        for condition in &self.conditions {
            writeln!(f, "    {}", condition)?;
        }
        Ok(())
    }
}

impl Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            When::CallingRoute { method, path } => write!(f, "calling {} {}", method, path),
            When::ExecutingPipeline { name } => write!(f, "executing pipeline {}", name),
            When::ExecutingVariable { var_type, name } => write!(f, "executing variable {} {}", var_type, name),
        }
    }
}

impl Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {} {}", self.condition_type, self.field, self.comparison, self.value)
    }
}

impl Display for ConditionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConditionType::Then => write!(f, "then"),
            ConditionType::And => write!(f, "and"),
        }
    }
}

// Multi-line string parser for backtick-delimited content
fn parse_multiline_string(input: &str) -> IResult<&str, String> {
    delimited(
        char('`'),
        map(take_until("`"), |s: &str| s.to_string()),
        char('`')
    ).parse(input)
}

// Basic parsers
fn parse_method(input: &str) -> IResult<&str, String> {
    map(
        alt((tag("GET"), tag("POST"), tag("PUT"), tag("DELETE"))),
        |s: &str| s.to_string()
    ).parse(input)
}

fn parse_identifier(input: &str) -> IResult<&str, String> {
    map(
        recognize((
            alt((alphanumeric1, tag("_"))),
            many0(alt((alphanumeric1, tag("_"), tag("-"))))
        )),
        |s: &str| s.to_string()
    ).parse(input)
}

// Config parsing
fn parse_config_value(input: &str) -> IResult<&str, ConfigValue> {
    alt((
        // Environment variable with default
        map(
            (
                preceded(char('$'), parse_identifier),
                preceded(
                    (multispace0, tag("||"), multispace0),
                    delimited(char('"'), take_until("\""), char('"'))
                )
            ),
            |(var, default)| ConfigValue::EnvVar { var, default: Some(default.to_string()) }
        ),
        // Environment variable without default
        map(
            preceded(char('$'), parse_identifier),
            |var| ConfigValue::EnvVar { var, default: None }
        ),
        // String literal
        map(
            delimited(char('"'), take_until("\""), char('"')),
            |s: &str| ConfigValue::String(s.to_string())
        ),
        // Boolean
        map(alt((tag("true"), tag("false"))), |s| {
            ConfigValue::Boolean(s == "true")
        }),
        // Number
        map_res(digit1, |s: &str| s.parse::<i64>().map(ConfigValue::Number))
    )).parse(input)
}

fn parse_config_property(input: &str) -> IResult<&str, ConfigProperty> {
    let (input, _) = multispace0(input)?;
    let (input, key) = parse_identifier(input)?;
    let (input, _) = (multispace0, char(':'), multispace0).parse(input)?;
    let (input, value) = parse_config_value(input)?;
    Ok((input, ConfigProperty { key, value }))
}

fn parse_config(input: &str) -> IResult<&str, Config> {
    let (input, _) = tag("config")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('{')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, properties) = many0(parse_config_property).parse(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('}')(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Config { name, properties }))
}

// Pipeline parsing
fn parse_pipeline_step(input: &str) -> IResult<&str, PipelineStep> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = char(':')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, config) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, PipelineStep { name, config }))
}

fn parse_pipeline(input: &str) -> IResult<&str, Pipeline> {
    map(many0(parse_pipeline_step), |steps| Pipeline { steps }).parse(input)
}

fn parse_named_pipeline(input: &str) -> IResult<&str, NamedPipeline> {
    let (input, _) = tag("pipeline")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, NamedPipeline { name, pipeline }))
}

fn parse_pipeline_ref(input: &str) -> IResult<&str, PipelineRef> {
    alt((
        // Named pipeline reference
        map(
            preceded(
                (multispace0, tag("|>"), multispace0, tag("pipeline:"), multispace0),
                parse_identifier
            ),
            PipelineRef::Named
        ),
        // Inline pipeline
        map(parse_pipeline, PipelineRef::Inline)
    )).parse(input)
}

// Variable parsing
fn parse_variable(input: &str) -> IResult<&str, Variable> {
    let (input, var_type) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Variable { var_type, name, value }))
}

// Route parsing
fn parse_route(input: &str) -> IResult<&str, Route> {
    let (input, method) = parse_method(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = take_till1(|c| c == ' ' || c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline_ref(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Route { 
        method, 
        path: path.to_string(), 
        pipeline 
    }))
}

// Test parsing
fn parse_when(input: &str) -> IResult<&str, When> {
    alt((
        map(
            (
                tag("calling"),
                multispace0,
                parse_method,
                multispace0,
                take_till1(|c| c == '\n')
            ),
            |(_, _, method, _, path)| When::CallingRoute { method, path: path.to_string() }
        ),
        map(
            (
                tag("executing"),
                multispace0,
                tag("pipeline"),
                multispace0,
                parse_identifier
            ),
            |(_, _, _, _, name)| When::ExecutingPipeline { name }
        ),
        map(
            (
                tag("executing"),
                multispace0,
                tag("variable"),
                multispace0,
                parse_identifier,
                multispace0,
                parse_identifier
            ),
            |(_, _, _, _, var_type, _, name)| When::ExecutingVariable { var_type, name }
        ),
    )).parse(input)
}

fn parse_condition(input: &str) -> IResult<&str, Condition> {
    let (input, _) = multispace0(input)?;
    let (input, condition_type_str) = alt((tag("then"), tag("and"))).parse(input)?;
    let condition_type = match condition_type_str {
        "then" => ConditionType::Then,
        "and" => ConditionType::And,
        _ => unreachable!(),
    };
    let (input, _) = multispace0(input)?;
    let (input, field) = take_till1(|c| c == ' ')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, comparison) = take_till1(|c| c == ' ')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = take_till1(|c| c == '\n')(input)?;
    Ok((input, Condition {
        condition_type,
        field: field.to_string(),
        comparison: comparison.to_string(),
        value: value.to_string(),
    }))
}

fn parse_mock(input: &str) -> IResult<&str, Mock> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("with")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("mock")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, target) = take_till1(|c| c == ' ')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("returning")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, return_value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Mock { 
        target: target.to_string(), 
        return_value 
    }))
}

fn parse_it(input: &str) -> IResult<&str, It> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("it")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('"')(input)?;
    let (input, name) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    let (input, _) = multispace0(input)?;
    
    // Parse optional per-test mocks
    let (input, mocks) = many0(parse_mock).parse(input)?;
    
    let (input, _) = tag("when")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, when) = parse_when(input)?;
    let (input, _) = multispace0(input)?;
    
    // Parse optional input
    let (input, input_opt) = opt(
        preceded(
            (tag("with"), multispace0, tag("input"), multispace0),
            parse_multiline_string
        )
    ).parse(input)?;
    let (input, _) = multispace0(input)?;
    
    let (input, conditions) = many0(parse_condition).parse(input)?;
    
    Ok((input, It { 
        name: name.to_string(), 
        mocks, 
        when, 
        input: input_opt, 
        conditions 
    }))
}

fn parse_describe(input: &str) -> IResult<&str, Describe> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("describe")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('"')(input)?;
    let (input, name) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    let (input, _) = multispace0(input)?;
    
    // Parse optional describe-level mocks
    let (input, mocks) = many0(parse_mock).parse(input)?;
    let (input, _) = multispace0(input)?;
    
    let (input, tests) = many0(parse_it).parse(input)?;
    
    Ok((input, Describe { 
        name: name.to_string(), 
        mocks, 
        tests 
    }))
}

// Top-level program parsing
pub fn parse_program(input: &str) -> IResult<&str, Program> {
    let (input, _) = multispace0(input)?;
    
    let mut configs = Vec::new();
    let mut pipelines = Vec::new();
    let mut variables = Vec::new();
    let mut routes = Vec::new();
    let mut describes = Vec::new();
    
    let mut remaining = input;
    
    while !remaining.is_empty() {
        let (new_remaining, _) = multispace0(remaining)?;
        if new_remaining.is_empty() {
            break;
        }
        
        // Try parsing each type of top-level element
        if let Ok((new_input, config)) = parse_config(new_remaining) {
            configs.push(config);
            remaining = new_input;
        } else if let Ok((new_input, pipeline)) = parse_named_pipeline(new_remaining) {
            pipelines.push(pipeline);
            remaining = new_input;
        } else if let Ok((new_input, variable)) = parse_variable(new_remaining) {
            variables.push(variable);
            remaining = new_input;
        } else if let Ok((new_input, route)) = parse_route(new_remaining) {
            routes.push(route);
            remaining = new_input;
        } else if let Ok((new_input, describe)) = parse_describe(new_remaining) {
            describes.push(describe);
            remaining = new_input;
        } else {
            // Skip unrecognized content
            if let Ok((new_input, _)) = take_till1::<_, _, nom::error::Error<&str>>(|c| c == '\n').parse(new_remaining) {
                remaining = new_input;
            } else {
                break;
            }
        }
    }
    
    Ok((remaining, Program { configs, pipelines, variables, routes, describes }))
}
