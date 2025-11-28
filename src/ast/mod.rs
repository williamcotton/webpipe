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
    pub graphql_schema: Option<GraphQLSchema>,
    pub queries: Vec<QueryResolver>,
    pub mutations: Vec<MutationResolver>,
    pub feature_flags: Option<Pipeline>,
}

#[derive(Debug, Clone)]
pub struct GraphQLSchema {
    pub sdl: String,
}

#[derive(Debug, Clone)]
pub struct QueryResolver {
    pub name: String,
    pub pipeline: Pipeline,
}

#[derive(Debug, Clone)]
pub struct MutationResolver {
    pub name: String,
    pub pipeline: Pipeline,
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
pub enum ConfigType {
    Backtick,
    Quoted,
    Identifier,
}

#[derive(Debug, Clone)]
pub struct Tag {
    /// Tag name, e.g. "prod", "dev", "async", "flag"
    pub name: String,

    /// True if tag was written as @!name
    pub negated: bool,

    /// Arguments inside parentheses, e.g. ["new-ui", "beta"] for @flag(new-ui,beta)
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum PipelineStep {
    Regular {
        name: String,
        config: String,
        config_type: ConfigType,
        tags: Vec<Tag>,
        /// Pre-parsed join task names (only populated for "join" middleware)
        /// This avoids parsing the config string on every request in the hot path.
        parsed_join_targets: Option<Vec<String>>,
    },
    Result { branches: Vec<ResultBranch> },
    If {
        condition: Pipeline,
        then_branch: Pipeline,
        else_branch: Option<Pipeline>
    },
}

#[derive(Debug, Clone)]
pub struct ResultBranch {
    pub branch_type: ResultBranchType,
    pub status_code: u16,
    pub pipeline: Pipeline,
}

#[derive(Debug, Clone)]
pub enum ResultBranchType {
    Ok,
    Custom(String),
    Default,
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
    pub jq_expr: Option<String>,
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
        if let Some(flags) = &self.feature_flags {
            writeln!(f, "featureFlags =")?;
            write!(f, "{}", flags)?;
            writeln!(f)?;
        }
        if let Some(schema) = &self.graphql_schema {
            writeln!(f, "{}", schema)?;
            writeln!(f)?;
        }
        for query in &self.queries {
            writeln!(f, "{}", query)?;
            writeln!(f)?;
        }
        for mutation in &self.mutations {
            writeln!(f, "{}", mutation)?;
            writeln!(f)?;
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

impl Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@")?;
        if self.negated {
            write!(f, "!")?;
        }
        write!(f, "{}", self.name)?;
        if !self.args.is_empty() {
            write!(f, "({})", self.args.join(","))?;
        }
        Ok(())
    }
}

impl Display for PipelineStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStep::Regular { name, config, config_type, tags, .. } => {
                let formatted_config = match config_type {
                    ConfigType::Backtick => format!("`{}`", config),
                    ConfigType::Quoted => format!("\"{}\"", config),
                    ConfigType::Identifier => config.clone(),
                };
                write!(f, "  |> {}: {}", name, formatted_config)?;

                // Append tags if any
                if !tags.is_empty() {
                    for tag in tags {
                        write!(f, " {}", tag)?;
                    }
                }
                Ok(())
            }
            PipelineStep::Result { branches } => {
                writeln!(f, "  |> result")?;
                for branch in branches {
                    write!(f, "    {}", branch)?;
                }
                Ok(())
            }
            PipelineStep::If { condition, then_branch, else_branch } => {
                writeln!(f, "  |> if")?;
                // Format condition pipeline with proper indentation
                for step in &condition.steps {
                    writeln!(f, "  {}", step)?;
                }
                writeln!(f, "    then:")?;
                // Format then branch with proper indentation
                for step in &then_branch.steps {
                    writeln!(f, "    {}", step)?;
                }
                // Format else branch if present
                if let Some(else_pipe) = else_branch {
                    writeln!(f, "    else:")?;
                    for step in &else_pipe.steps {
                        writeln!(f, "    {}", step)?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Display for ResultBranch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let branch_name = match &self.branch_type {
            ResultBranchType::Ok => "ok".to_string(),
            ResultBranchType::Custom(name) => name.clone(),
            ResultBranchType::Default => "default".to_string(),
        };
        writeln!(f, "{}({}):", branch_name, self.status_code)?;
        let pipeline_str = format!("{}", self.pipeline);
        for line in pipeline_str.lines() {
            writeln!(f, "    {}", line)?;
        }
        Ok(())
    }
}

impl Display for ResultBranchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResultBranchType::Ok => write!(f, "ok"),
            ResultBranchType::Custom(name) => write!(f, "{}", name),
            ResultBranchType::Default => write!(f, "default"),
        }
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
        let field_with_jq = if let Some(expr) = &self.jq_expr {
            format!("{} `{}`", self.field, expr)
        } else {
            self.field.clone()
        };
        write!(f, "{} {} {} {}", self.condition_type, field_with_jq, self.comparison, self.value)
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

impl Display for GraphQLSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "graphqlSchema = `{}`", self.sdl)
    }
}

impl Display for QueryResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "query {} =", self.name)?;
        write!(f, "{}", self.pipeline)
    }
}

impl Display for MutationResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "mutation {} =", self.name)?;
        write!(f, "{}", self.pipeline)
    }
}

// Skip whitespace and comments (lines starting with #)
fn skip_ws_and_comments(input: &str) -> IResult<&str, ()> {
    let mut remaining = input;
    loop {
        // Skip whitespace
        let (new_input, _) = multispace0(remaining)?;
        remaining = new_input;
        
        // Check for comment line
        if remaining.starts_with('#') {
            // Skip to end of line
            if let Some(newline_pos) = remaining.find('\n') {
                remaining = &remaining[newline_pos + 1..];
            } else {
                // Comment goes to end of input
                remaining = "";
            }
        } else {
            break;
        }
    }
    Ok((remaining, ()))
}

// Multi-line string parser for backtick-delimited content
fn parse_multiline_string(input: &str) -> IResult<&str, String> {
    delimited(
        char('`'),
        map(take_until("`"), |s: &str| s.to_string()),
        char('`')
    ).parse(input)
}

// Pipeline step configuration parser - handles both backticks and double quotes
fn parse_step_config(input: &str) -> IResult<&str, (String, ConfigType)> {
    alt((
        // Backtick-delimited multi-line strings
        map(
            delimited(
                char('`'),
                map(take_until("`"), |s: &str| s.to_string()),
                char('`')
            ),
            |s| (s, ConfigType::Backtick)
        ),
        // Double quote-delimited strings
        map(
            delimited(
                char('"'),
                map(take_until("\""), |s: &str| s.to_string()),
                char('"')
            ),
            |s| (s, ConfigType::Quoted)
        ),
        // Bare identifier (variable reference like: pg: getUserQuery)
        map(parse_identifier, |s| (s, ConfigType::Identifier))
    )).parse(input)
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
fn parse_if_step(input: &str) -> IResult<&str, PipelineStep> {
    // 1. Parse Header: "|> if"
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("if")(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 2. Parse Condition Pipeline
    // This works because parse_pipeline stops at tokens not starting with "|>"
    let (input, condition) = parse_pipeline(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 3. Parse "then:" block
    let (input, _) = tag("then:")(input)?;
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, then_branch) = parse_pipeline(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 4. Parse Optional "else:" block
    let (input, else_branch) = opt(preceded(
        |i| {
            let (i, _) = tag("else:")(i)?;
            skip_ws_and_comments(i)
        },
        parse_pipeline
    )).parse(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 5. Parse Optional "end" keyword
    let (input, _) = opt(tag("end")).parse(input)?;

    Ok((input, PipelineStep::If {
        condition,
        then_branch,
        else_branch,
    }))
}

fn parse_pipeline_step(input: &str) -> IResult<&str, PipelineStep> {
    alt((
        parse_result_step,
        parse_if_step,
        parse_regular_step
    )).parse(input)
}

// Tag parsing
fn parse_tag(input: &str) -> IResult<&str, Tag> {
    let (input, _) = char('@')(input)?;

    // Check for negation
    let (input, negated) = opt(char('!')).parse(input)?;
    let negated = negated.is_some();

    // Parse tag name
    let (input, name) = parse_identifier(input)?;

    // Parse optional arguments
    let (input, args) = opt(parse_tag_args).parse(input)?;
    let args = args.unwrap_or_default();

    Ok((input, Tag { name, negated, args }))
}

fn parse_tag_args(input: &str) -> IResult<&str, Vec<String>> {
    delimited(
        char('('),
        nom::multi::separated_list1(
            char(','),
            parse_identifier
        ),
        char(')')
    ).parse(input)
}

fn parse_tags(input: &str) -> IResult<&str, Vec<Tag>> {
    many0(
        preceded(
            // Skip inline spaces before each tag
            nom::bytes::complete::take_while(|c| c == ' ' || c == '\t'),
            parse_tag
        )
    ).parse(input)
}

/// Pre-parse join config into task names at AST parse time.
/// This avoids repeated parsing in the hot path during execution.
fn parse_join_task_names(config: &str) -> Option<Vec<String>> {
    let trimmed = config.trim();

    // Try parsing as JSON array first
    if trimmed.starts_with('[') {
        if let Ok(names) = serde_json::from_str::<Vec<String>>(trimmed) {
            return Some(names);
        }
        return None; // Invalid JSON, will error at runtime
    }

    // Otherwise parse as comma-separated list
    let names: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if names.is_empty() {
        return None; // Will error at runtime
    }

    Some(names)
}

fn parse_regular_step(input: &str) -> IResult<&str, PipelineStep> {
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = char(':')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, (config, config_type)) = parse_step_config(input)?;

    // Parse tags (before consuming trailing whitespace)
    let (input, tags) = parse_tags(input)?;

    // Pre-parse join targets for join middleware (compile-time optimization)
    let parsed_join_targets = if name == "join" {
        parse_join_task_names(&config)
    } else {
        None
    };

    let (input, _) = multispace0(input)?;
    Ok((input, PipelineStep::Regular { name, config, config_type, tags, parsed_join_targets }))
}

fn parse_result_step(input: &str) -> IResult<&str, PipelineStep> {
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("result")(input)?;
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, branches) = many0(parse_result_branch).parse(input)?;
    Ok((input, PipelineStep::Result { branches }))
}

fn parse_result_branch(input: &str) -> IResult<&str, ResultBranch> {
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, branch_type_str) = parse_identifier(input)?;
    let branch_type = match branch_type_str.as_str() {
        "ok" => ResultBranchType::Ok,
        "default" => ResultBranchType::Default,
        _ => ResultBranchType::Custom(branch_type_str),
    };
    let (input, _) = char('(')(input)?;
    let (input, status_code_str) = digit1(input)?;
    let status_code = status_code_str.parse::<u16>().map_err(|_| {
        nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
    })?;
    let (input, _) = char(')')(input)?;
    let (input, _) = char(':')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    Ok((input, ResultBranch { branch_type, status_code, pipeline }))
}

fn parse_pipeline(input: &str) -> IResult<&str, Pipeline> {
    let mut steps = Vec::new();
    let mut remaining = input;
    
    loop {
        // Skip whitespace and comments before checking for next step
        let (new_input, _) = skip_ws_and_comments(remaining)?;
        
        // Try to parse a pipeline step
        if let Ok((after_step, step)) = parse_pipeline_step(new_input) {
            steps.push(step);
            remaining = after_step;
        } else {
            // No more steps, return what we have
            // Don't consume the whitespace/comments if there's no step
            return Ok((remaining, Pipeline { steps }));
        }
    }
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

fn parse_feature_flags(input: &str) -> IResult<&str, Pipeline> {
    let (input, _) = tag("featureFlags")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, pipeline))
}

fn parse_pipeline_ref(input: &str) -> IResult<&str, PipelineRef> {
    alt((
        // Inline pipeline (prefer inline so routes can chain steps after a pipeline call)
        map(parse_pipeline, PipelineRef::Inline),
        // Named pipeline reference (single step)
        map(
            preceded(
                (multispace0, tag("|>"), multispace0, tag("pipeline:"), multispace0),
                parse_identifier
            ),
            PipelineRef::Named
        )
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

// GraphQL parsing
fn parse_graphql_schema(input: &str) -> IResult<&str, GraphQLSchema> {
    let (input, _) = tag("graphqlSchema")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, sdl) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, GraphQLSchema { sdl }))
}

fn parse_query_resolver(input: &str) -> IResult<&str, QueryResolver> {
    let (input, _) = tag("query")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, QueryResolver { name, pipeline }))
}

fn parse_mutation_resolver(input: &str) -> IResult<&str, MutationResolver> {
    let (input, _) = tag("mutation")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, MutationResolver { name, pipeline }))
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
    // Optional backticked jq filter following the field
    let (input, jq_expr_opt) = opt(parse_multiline_string).parse(input)?;
    let (input, _) = multispace0(input)?;
    let (input, comparison) = take_till1(|c| c == ' ')(input)?;
    let (input, _) = multispace0(input)?;
    // Value can be a backtick-delimited multi-line string, a quoted string, or the rest of the line
    let (input, value) = alt((
        // backtick multi-line
        map(parse_multiline_string, |s| s.to_string()),
        // quoted on the same line
        map(
            delimited(char('"'), take_until("\""), char('"')),
            |s: &str| s.to_string()
        ),
        // bare until end of line
        map(take_till1(|c| c == '\n'), |s: &str| s.to_string()),
    )).parse(input)?;
    Ok((input, Condition {
        condition_type,
        field: field.to_string(),
        jq_expr: jq_expr_opt,
        comparison: comparison.to_string(),
        value,
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

fn parse_and_mock(input: &str) -> IResult<&str, Mock> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("and")(input)?;
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
    
    // Parse optional additional mocks starting with 'and mock'
    let (input, extra_mocks) = many0(parse_and_mock).parse(input)?;
    let (input, _) = multispace0(input)?;

    // Parse remaining conditions
    let (input, conditions) = many0(parse_condition).parse(input)?;
    
    let mut all_mocks = mocks;
    all_mocks.extend(extra_mocks);
    
    Ok((input, It { 
        name: name.to_string(), 
        mocks: all_mocks, 
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
    let mut graphql_schema = None;
    let mut queries = Vec::new();
    let mut mutations = Vec::new();
    let mut feature_flags = None;

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
        } else if let Ok((new_input, flags)) = parse_feature_flags(new_remaining) {
            feature_flags = Some(flags);
            remaining = new_input;
        } else if let Ok((new_input, schema)) = parse_graphql_schema(new_remaining) {
            graphql_schema = Some(schema);
            remaining = new_input;
        } else if let Ok((new_input, query)) = parse_query_resolver(new_remaining) {
            queries.push(query);
            remaining = new_input;
        } else if let Ok((new_input, mutation)) = parse_mutation_resolver(new_remaining) {
            mutations.push(mutation);
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

    Ok((remaining, Program { configs, pipelines, variables, routes, describes, graphql_schema, queries, mutations, feature_flags }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_program_and_roundtrip_display() {
        let src = r#"
config cache {
  enabled: true
  defaultTtl: 60
}

pipeline simple =
  |> handlebars: `Hello`

GET /
  |> pipeline: simple

describe "home"
  it "returns html"
    when calling GET /
    then status is 200
"#;
        let (_rest, program) = parse_program(src).unwrap();
        // sanity checks
        assert_eq!(program.configs.len(), 1);
        assert_eq!(program.pipelines.len(), 1);
        assert_eq!(program.routes.len(), 1);
        assert_eq!(program.describes.len(), 1);

        // Display should not panic and include route path
        let s = format!("{}", program);
        assert!(s.contains("GET /"));
    }

    #[test]
    fn parse_pipeline_steps_with_different_config_types() {
        let src = r#"
pipeline test =
  |> handlebars: `multi
line
template`
  |> jq: "{ hello: world }"
  |> echo: myVariable
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.pipelines.len(), 1);
        
        let pipeline = &program.pipelines[0].pipeline;
        assert_eq!(pipeline.steps.len(), 3);
        
        // Test backtick config type
        if let PipelineStep::Regular { name, config, config_type, .. } = &pipeline.steps[0] {
            assert_eq!(name, "handlebars");
            assert_eq!(config, "multi\nline\ntemplate");
            assert!(matches!(config_type, ConfigType::Backtick));
        } else {
            panic!("Expected Regular step");
        }

        // Test quoted config type
        if let PipelineStep::Regular { name, config, config_type, .. } = &pipeline.steps[1] {
            assert_eq!(name, "jq");
            assert_eq!(config, "{ hello: world }");
            assert!(matches!(config_type, ConfigType::Quoted));
        } else {
            panic!("Expected Regular step");
        }

        // Test identifier config type
        if let PipelineStep::Regular { name, config, config_type, .. } = &pipeline.steps[2] {
            assert_eq!(name, "echo");
            assert_eq!(config, "myVariable");
            assert!(matches!(config_type, ConfigType::Identifier));
        } else {
            panic!("Expected Regular step");
        }
        
        // Test roundtrip display preserves config type formatting
        let displayed = format!("{}", pipeline);
        assert!(displayed.contains("`multi\nline\ntemplate`"));
        assert!(displayed.contains("\"{ hello: world }\""));
        assert!(displayed.contains("myVariable")); // identifier without quotes
    }

    #[test]
    fn parse_tags_on_regular_steps() {
        let src = r#"
pipeline test =
  |> jq: `{ hello: "world" }` @prod
  |> pg: `SELECT * FROM users` @dev @flag(new-ui)
  |> log: `level: debug` @!prod @async(user)
  |> echo: myVar @flag(beta,staff)
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.pipelines.len(), 1);

        let pipeline = &program.pipelines[0].pipeline;
        assert_eq!(pipeline.steps.len(), 4);

        // Step 0: @prod
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[0] {
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].name, "prod");
            assert_eq!(tags[0].negated, false);
            assert_eq!(tags[0].args.len(), 0);
        } else {
            panic!("Expected Regular step");
        }

        // Step 1: @dev @flag(new-ui)
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[1] {
            assert_eq!(tags.len(), 2);
            assert_eq!(tags[0].name, "dev");
            assert_eq!(tags[1].name, "flag");
            assert_eq!(tags[1].args, vec!["new-ui"]);
        } else {
            panic!("Expected Regular step");
        }

        // Step 2: @!prod @async(user)
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[2] {
            assert_eq!(tags.len(), 2);
            assert_eq!(tags[0].name, "prod");
            assert_eq!(tags[0].negated, true);
            assert_eq!(tags[1].name, "async");
            assert_eq!(tags[1].args, vec!["user"]);
        } else {
            panic!("Expected Regular step");
        }

        // Step 3: @flag(beta,staff)
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[3] {
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].name, "flag");
            assert_eq!(tags[0].args, vec!["beta", "staff"]);
        } else {
            panic!("Expected Regular step");
        }

        // Test roundtrip
        let displayed = format!("{}", pipeline);
        assert!(displayed.contains("@prod"));
        assert!(displayed.contains("@dev @flag(new-ui)"));
        assert!(displayed.contains("@!prod @async(user)"));
        assert!(displayed.contains("@flag(beta,staff)"));
    }

    #[test]
    fn parse_steps_without_tags() {
        let src = r#"
pipeline test =
  |> jq: `.`
  |> echo: foo
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let pipeline = &program.pipelines[0].pipeline;

        // Steps without tags should have empty tags vec
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[0] {
            assert_eq!(tags.len(), 0);
        }
        if let PipelineStep::Regular { tags, .. } = &pipeline.steps[1] {
            assert_eq!(tags.len(), 0);
        }
    }

    #[test]
    fn format_tags_correctly() {
        let src = r#"
pipeline test =
  |> jq: `.` @prod @dev
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let formatted = format!("{}", program);

        assert!(formatted.contains("@prod @dev"));
    }

    #[test]
    fn format_negated_tag() {
        let src = r#"
pipeline test =
  |> jq: `.` @!prod
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let formatted = format!("{}", program);

        assert!(formatted.contains("@!prod"));
    }

    #[test]
    fn format_tag_with_args() {
        let src = r#"
pipeline test =
  |> jq: `.` @flag(a,b,c)
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let formatted = format!("{}", program);

        assert!(formatted.contains("@flag(a,b,c)"));
    }

    #[test]
    fn result_steps_do_not_have_tags() {
        let src = r#"
pipeline test =
  |> result
    ok(200):
      |> jq: `{ok: true}`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let pipeline = &program.pipelines[0].pipeline;
        let step = &pipeline.steps[0];

        assert!(matches!(step, PipelineStep::Result { .. }));
    }

    #[test]
    fn parse_graphql_schema() {
        let src = r#"
graphqlSchema = `
  type Query {
    hello: String
  }
`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert!(program.graphql_schema.is_some());
        let schema = program.graphql_schema.unwrap();
        assert!(schema.sdl.contains("hello"));
        assert!(schema.sdl.contains("Query"));
    }

    #[test]
    fn parse_query_resolver() {
        let src = r#"
query todos =
  |> auth: "required"
  |> jq: `{}`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.queries.len(), 1);
        assert_eq!(program.queries[0].name, "todos");
        assert_eq!(program.queries[0].pipeline.steps.len(), 2);
    }

    #[test]
    fn parse_mutation_resolver() {
        let src = r#"
mutation createTodo =
  |> auth: "required"
  |> jq: `{ title: .title }`
  |> pg: `INSERT INTO todos (title) VALUES ($1) RETURNING *`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.mutations.len(), 1);
        assert_eq!(program.mutations[0].name, "createTodo");
        assert_eq!(program.mutations[0].pipeline.steps.len(), 3);
    }

    #[test]
    fn roundtrip_graphql_constructs() {
        let src = r#"
graphqlSchema = `
  type Query {
    hello: String
  }
`

query hello =
  |> jq: `{ hello: "world" }`

mutation update =
  |> jq: `{ updated: true }`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let formatted = format!("{}", program);
        assert!(formatted.contains("graphqlSchema"));
        assert!(formatted.contains("query hello"));
        assert!(formatted.contains("mutation update"));
        assert!(formatted.contains("type Query"));
    }

    #[test]
    fn parse_comments_inside_pipeline() {
        let src = r#"
GET /test
  # comment before first step
  |> jq: `{ hello: "world" }`
  # comment between steps
  |> handlebars: `<p>{{hello}}</p>`
  # comment after last step
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.routes.len(), 1);
        
        // Verify all steps were parsed despite comments
        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            assert_eq!(pipeline.steps.len(), 2);
        } else {
            panic!("Expected inline pipeline");
        }
    }

    #[test]
    fn parse_comments_inside_if_blocks() {
        let src = r#"
GET /test-if
  |> jq: `{ level: 10 }`
  # comment before if
  |> if
    |> jq: `.level > 5`
    # comment before then
    then:
      # comment inside then
      |> jq: `. + { status: "high" }`
    # comment before else
    else:
      # comment inside else
      |> jq: `. + { status: "low" }`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.routes.len(), 1);
        
        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            assert_eq!(pipeline.steps.len(), 2); // jq + if
            
            // Verify the if step was parsed correctly
            if let PipelineStep::If { condition, then_branch, else_branch } = &pipeline.steps[1] {
                assert_eq!(condition.steps.len(), 1);
                assert_eq!(then_branch.steps.len(), 1);
                assert!(else_branch.is_some());
                assert_eq!(else_branch.as_ref().unwrap().steps.len(), 1);
            } else {
                panic!("Expected If step");
            }
        } else {
            panic!("Expected inline pipeline");
        }
    }

    #[test]
    fn parse_complete_graphql_program() {
        let src = r#"
config cache {
  enabled: true
}

graphqlSchema = `
  type Todo {
    id: ID!
    title: String!
  }

  type Query {
    todos: [Todo!]!
  }

  type Mutation {
    createTodo(title: String!): Todo!
  }
`

query todos =
  |> auth: "required"
  |> pg: `SELECT * FROM todos`
  |> jq: `.data.rows`

mutation createTodo =
  |> auth: "required"
  |> jq: `{ sqlParams: [.title] }`
  |> pg: `INSERT INTO todos (title) VALUES ($1) RETURNING *`
  |> jq: `.data.rows[0]`

GET /graphql
  |> graphql: `query { todos { id title } }`
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.configs.len(), 1);
        assert!(program.graphql_schema.is_some());
        assert_eq!(program.queries.len(), 1);
        assert_eq!(program.mutations.len(), 1);
        assert_eq!(program.routes.len(), 1);
    }
}
