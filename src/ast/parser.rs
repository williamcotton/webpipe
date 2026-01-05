use nom::IResult;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till1, take_until, take},
    character::complete::{alphanumeric1, char, digit1, multispace0},
    combinator::{map, map_res, opt, recognize},
    multi::many0,
    sequence::{delimited, preceded},
    Parser,
};

use super::types::*;
use nom_locate::LocatedSpan;

/// Span type that tracks location information
pub type Span<'a> = LocatedSpan<&'a str>;

/// Extract SourceLocation from a Span
fn location_from_span(span: Span) -> SourceLocation {
    SourceLocation {
        line: span.location_line() as usize,
        column: span.get_column(),
        offset: span.location_offset(),
        file_path: None,
        module_id: None,
    }
}

// Skip whitespace and comments (lines starting with #)
fn skip_ws_and_comments(input: Span) -> IResult<Span, ()> {
    let mut remaining = input;
    loop {
        // Skip whitespace
        let (new_input, _) = multispace0(remaining)?;
        remaining = new_input;

        // Check for comment line
        let fragment = remaining.fragment();
        if fragment.starts_with('#') {
            // Skip to end of line
            if let Some(newline_pos) = fragment.find('\n') {
                let (new_input, _) = take(newline_pos + 1)(remaining)?;
                remaining = new_input;
            } else {
                // Comment goes to end of input
                let (new_input, _) = take(fragment.len())(remaining)?;
                remaining = new_input;
            }
        } else {
            break;
        }
    }
    Ok((remaining, ()))
}

// Multi-line string parser for backtick-delimited content
fn parse_multiline_string(input: Span) -> IResult<Span, String> {
    delimited(
        char('`'),
        map(take_until("`"), |s: Span| s.fragment().to_string()),
        char('`')
    ).parse(input)
}

// Pipeline step configuration parser - handles both backticks and double quotes
fn parse_step_config(input: Span) -> IResult<Span, (String, ConfigType)> {
    alt((
        // Backtick-delimited multi-line strings
        map(
            delimited(
                char('`'),
                map(take_until("`"), |s: Span| s.fragment().to_string()),
                char('`')
            ),
            |s| (s, ConfigType::Backtick)
        ),
        // Double quote-delimited strings
        map(
            delimited(
                char('"'),
                map(take_until("\""), |s: Span| s.fragment().to_string()),
                char('"')
            ),
            |s| (s, ConfigType::Quoted)
        ),
        // Bare identifier (variable reference like: pg: getUserQuery or scoped: auth::login)
        map(parse_scoped_identifier_string, |s| (s, ConfigType::Identifier))
    )).parse(input)
}

// Basic parsers
fn parse_method(input: Span) -> IResult<Span, String> {
    map(
        alt((tag("GET"), tag("POST"), tag("PUT"), tag("DELETE"))),
        |s: Span| s.fragment().to_string()
    ).parse(input)
}

fn parse_identifier(input: Span) -> IResult<Span, String> {
    map(
        recognize((
            alt((alphanumeric1, tag("_"))),
            many0(alt((alphanumeric1, tag("_"), tag("-"))))
        )),
        |s: Span| s.fragment().to_string()
    ).parse(input)
}

// Config parsing
fn parse_config_value(input: Span) -> IResult<Span, ConfigValue> {
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
            |(var, default)| ConfigValue::EnvVar { var, default: Some(default.fragment().to_string()) }
        ),
        // Environment variable without default
        map(
            preceded(char('$'), parse_identifier),
            |var| ConfigValue::EnvVar { var, default: None }
        ),
        // String literal
        map(
            delimited(char('"'), take_until("\""), char('"')),
            |s: Span| ConfigValue::String(s.fragment().to_string())
        ),
        // Boolean
        map(alt((tag("true"), tag("false"))), |s: Span| {
            ConfigValue::Boolean(s.fragment() == &"true")
        }),
        // Number
        map_res(digit1, |s: Span| s.fragment().parse::<i64>().map(ConfigValue::Number))
    )).parse(input)
}

fn parse_config_property(input: Span) -> IResult<Span, ConfigProperty> {
    let (input, _) = multispace0(input)?;
    let (input, key) = parse_identifier(input)?;
    let (input, _) = (multispace0, char(':'), multispace0).parse(input)?;
    let (input, value) = parse_config_value(input)?;
    Ok((input, ConfigProperty { key, value }))
}

fn parse_config(input: Span) -> IResult<Span, Config> {
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
fn parse_if_step(input: Span) -> IResult<Span, PipelineStep> {
    // 1. Parse Header: "|> if"
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
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

    let location = location_from_span(start_span);

    Ok((input, PipelineStep::If {
        condition,
        then_branch,
        else_branch,
        location,
    }))
}

// Parse a tag expression with boolean operators (and, or) and grouping
// Grammar (precedence: AND > OR):
//   tag_expr := or_expr
//   or_expr  := and_expr ("or" and_expr)*
//   and_expr := primary ("and" primary)*
//   primary  := tag | "(" tag_expr ")"
fn parse_tag_expr(input: Span) -> IResult<Span, TagExpr> {
    parse_or_expr(input)
}

fn parse_or_expr(input: Span) -> IResult<Span, TagExpr> {
    let (input, first) = parse_and_expr(input)?;
    let (input, rest) = many0(preceded(
        delimited(multispace0, tag("or"), multispace0),
        parse_and_expr
    )).parse(input)?;

    Ok((input, rest.into_iter().fold(first, |acc, expr| {
        TagExpr::Or(Box::new(acc), Box::new(expr))
    })))
}

fn parse_and_expr(input: Span) -> IResult<Span, TagExpr> {
    let (input, first) = parse_tag_primary(input)?;
    let (input, rest) = many0(preceded(
        delimited(multispace0, tag("and"), multispace0),
        parse_tag_primary
    )).parse(input)?;

    Ok((input, rest.into_iter().fold(first, |acc, expr| {
        TagExpr::And(Box::new(acc), Box::new(expr))
    })))
}

fn parse_tag_primary(input: Span) -> IResult<Span, TagExpr> {
    alt((
        // Grouped expression: ( expr )
        map(
            delimited(
                (char('('), multispace0),
                parse_tag_expr,
                (multispace0, char(')'))
            ),
            |expr| expr
        ),
        // Single tag
        map(parse_tag, TagExpr::Tag)
    )).parse(input)
}

// Parse a single dispatch branch: case @tag: |> pipeline
// Now supports boolean expressions: case @when(a) and @when(b):
fn parse_dispatch_branch(input: Span) -> IResult<Span, DispatchBranch> {
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, _) = tag("case")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, condition) = parse_tag_expr(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char(':')(input)?;
    let (input, _) = skip_ws_and_comments(input)?;
    // parse_pipeline stops when it hits tokens it doesn't recognize
    let (input, pipeline) = parse_pipeline(input)?;
    let location = location_from_span(start_span);
    Ok((input, DispatchBranch { condition, pipeline, location }))
}

// Parse the dispatch step: |> dispatch case ... case ... default: ... end
fn parse_dispatch_step(input: Span) -> IResult<Span, PipelineStep> {
    // 1. Parse Header: "|> dispatch"
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("dispatch")(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 2. Parse 'case' branches
    let (input, branches) = many0(parse_dispatch_branch).parse(input)?;

    // 3. Parse optional 'default:' branch
    let (input, default) = opt(preceded(
        |i| {
            let (i, _) = skip_ws_and_comments(i)?;
            let (i, _) = tag("default:")(i)?;
            skip_ws_and_comments(i)
        },
        parse_pipeline
    )).parse(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 4. Parse optional 'end' keyword
    let (input, _) = opt(tag("end")).parse(input)?;

    let location = location_from_span(start_span);

    Ok((input, PipelineStep::Dispatch { branches, default, location }))
}

fn parse_foreach_step(input: Span) -> IResult<Span, PipelineStep> {
    // 1. Header: "|> foreach"
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("foreach")(input)?;
    let (input, _) = nom::character::complete::multispace1(input)?; // Enforce space

    // 2. Selector: "data.rows"
    // Consume characters until we hit a newline or a comment start (#)
    let (input, raw_selector) = take_till1(|c| c == '\n' || c == '#')(input)?;
    let selector = raw_selector.fragment().trim().to_string();
    let (input, _) = skip_ws_and_comments(input)?;

    // 3. Inner Pipeline
    // Recursively parse the block content
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // 4. Footer: "end"
    let (input, _) = tag("end")(input)?;

    let location = location_from_span(start_span);

    Ok((input, PipelineStep::Foreach {
        selector,
        pipeline,
        location,
    }))
}

fn parse_pipeline_step(input: Span) -> IResult<Span, PipelineStep> {
    alt((
        parse_result_step,
        parse_if_step,
        parse_dispatch_step,
        parse_foreach_step,
        parse_regular_step
    )).parse(input)
}

// Tag parsing
fn parse_tag(input: Span) -> IResult<Span, Tag> {
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

// Parse a tag argument - can be either an identifier or a backtick string
fn parse_tag_argument(input: Span) -> IResult<Span, String> {
    alt((
        // Allow backtick strings for JQ expressions (like @guard)
        parse_multiline_string,
        // Allow standard identifiers (existing behavior)
        parse_identifier
    )).parse(input)
}

fn parse_tag_args(input: Span) -> IResult<Span, Vec<String>> {
    delimited(
        char('('),
        nom::multi::separated_list1(
            (char(','), multispace0),
            preceded(multispace0, parse_tag_argument)
        ),
        char(')')
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

/// Parse optional step condition (tag expression after the config)
/// Supports:
///   - @tag (single tag)
///   - @tag @tag2 (implicit AND for backwards compatibility)
///   - @tag and @tag2 (explicit AND)
///   - @tag or @tag2 (explicit OR)
///   - (@tag or @tag2) and @tag3 (grouping)
fn parse_step_condition(input: Span) -> IResult<Span, Option<TagExpr>> {
    // Skip inline whitespace
    let (input, _) = nom::bytes::complete::take_while(|c| c == ' ' || c == '\t')(input)?;

    // Check if there's a tag expression starting (@ for tags, ( for grouped expressions)
    let fragment = input.fragment();
    if !fragment.starts_with('@') && !fragment.starts_with('(') {
        return Ok((input, None));
    }

    // Parse the first tag expression (which may include and/or)
    let (mut input, mut expr) = parse_tag_expr(input)?;

    // Check for additional space-separated tags (implicit AND for backwards compatibility)
    // This handles: @dev @flag(x) which was valid in the old Vec<Tag> format
    loop {
        let (remaining, _) = nom::bytes::complete::take_while(|c| c == ' ' || c == '\t')(input)?;

        // Check if there's another tag starting (without and/or keyword)
        let frag = remaining.fragment();
        if !frag.starts_with('@') {
            break;
        }

        // Make sure it's not just a newline or comment coming
        if frag.starts_with('\n') || frag.starts_with('#') || frag.starts_with("//") {
            break;
        }

        // Parse the next tag (just a single tag, not a full expression)
        let (remaining, next_tag) = parse_tag(remaining)?;
        expr = TagExpr::And(Box::new(expr), Box::new(TagExpr::Tag(next_tag)));
        input = remaining;
    }

    Ok((input, Some(expr)))
}

/// Parse inline arguments for middleware (e.g., fetch(url, options))
/// Returns a vector of argument strings
fn parse_inline_args(input: Span) -> IResult<Span, Vec<String>> {
    // Check if we have arguments starting with ( or [
    let fragment = input.fragment();
    let trimmed_input = fragment.trim_start();
    if !trimmed_input.starts_with('(') && !trimmed_input.starts_with('[') {
        return Ok((input, Vec::new()));
    }

    let open_char = if trimmed_input.starts_with('(') { '(' } else { '[' };
    let close_char = if open_char == '(' { ')' } else { ']' };

    // Consume the opening bracket
    let (input, _) = nom::character::complete::char(open_char)(input)?;

    // Find the balanced closing bracket and extract the content
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut end_pos = None;

    let input_fragment = input.fragment();
    for (idx, ch) in input_fragment.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            c if c == open_char && !in_string => depth += 1,
            c if c == close_char && !in_string => {
                if depth == 0 {
                    end_pos = Some(idx);
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    let end_pos = end_pos.ok_or_else(|| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Eof))
    })?;

    let args_content = &input_fragment[..end_pos];
    let (remaining, _) = take(end_pos + 1)(input)?; // Skip the closing bracket

    // Split by commas, respecting nesting and strings
    let args = split_balanced_args(args_content);

    Ok((remaining, args))
}

/// Split argument string by commas while respecting nesting and strings
fn split_balanced_args(content: &str) -> Vec<String> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    let mut args = Vec::new();
    let mut current_arg = String::new();
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for ch in content.chars() {
        if escape_next {
            current_arg.push(ch);
            escape_next = false;
            continue;
        }

        match ch {
            '\\' if in_string => {
                current_arg.push(ch);
                escape_next = true;
            }
            '"' => {
                current_arg.push(ch);
                in_string = !in_string;
            }
            '(' | '[' | '{' if !in_string => {
                current_arg.push(ch);
                depth += 1;
            }
            ')' | ']' | '}' if !in_string => {
                current_arg.push(ch);
                depth -= 1;
            }
            ',' if depth == 0 && !in_string => {
                // This comma is a separator
                args.push(current_arg.trim().to_string());
                current_arg.clear();
            }
            _ => current_arg.push(ch),
        }
    }

    // Push the last argument
    if !current_arg.trim().is_empty() {
        args.push(current_arg.trim().to_string());
    }

    args
}

fn parse_regular_step(input: Span) -> IResult<Span, PipelineStep> {
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;

    // Parse optional inline arguments (e.g., fetch(url, options))
    let (input, args) = parse_inline_args(input)?;

    let (input, _) = multispace0(input)?;

    // Parse optional config (colon followed by config)
    let (input, config_pair) = opt(|i| {
        let (i, _) = char(':')(i)?;
        let (i, _) = multispace0(i)?;
        parse_step_config(i)
    }).parse(input)?;

    let (config, config_type) = config_pair.unwrap_or((String::new(), ConfigType::Quoted));

    // Parse optional condition (tag expression)
    let (input, condition) = parse_step_condition(input)?;

    // Pre-parse join targets for join middleware (compile-time optimization)
    let parsed_join_targets = if name == "join" {
        parse_join_task_names(&config)
    } else {
        None
    };

    let (input, _) = multispace0(input)?;
    let location = location_from_span(start_span);
    Ok((input, PipelineStep::Regular { name, args, config, config_type, condition, parsed_join_targets, location }))
}

fn parse_result_step(input: Span) -> IResult<Span, PipelineStep> {
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, _) = tag("|>")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("result")(input)?;
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, branches) = many0(parse_result_branch).parse(input)?;
    let location = location_from_span(start_span);
    Ok((input, PipelineStep::Result { branches, location }))
}

fn parse_result_branch(input: Span) -> IResult<Span, ResultBranch> {
    let (input, _) = skip_ws_and_comments(input)?;
    let start_span = input;
    let (input, branch_type_str) = parse_identifier(input)?;
    let branch_type = match branch_type_str.as_str() {
        "ok" => ResultBranchType::Ok,
        "default" => ResultBranchType::Default,
        _ => ResultBranchType::Custom(branch_type_str),
    };
    let (input, _) = char('(')(input)?;
    let (input, status_code_str) = digit1(input)?;
    let status_code = status_code_str.fragment().parse::<u16>().map_err(|_| {
        nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
    })?;
    let (input, _) = char(')')(input)?;
    let (input, _) = char(':')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let location = location_from_span(start_span);
    Ok((input, ResultBranch { branch_type, status_code, pipeline, location }))
}

fn parse_pipeline(input: Span) -> IResult<Span, Pipeline> {
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

fn parse_named_pipeline(input: Span) -> IResult<Span, NamedPipeline> {
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

fn parse_feature_flags(input: Span) -> IResult<Span, Pipeline> {
    let (input, _) = tag("featureFlags")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, pipeline))
}

fn parse_pipeline_ref(input: Span) -> IResult<Span, PipelineRef> {
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
// Import parsing
// Syntax: import "./path/to/file.wp" as alias
fn parse_import(input: Span) -> IResult<Span, Import> {
    let start_location = location_from_span(input);
    let (input, _) = tag("import")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = delimited(
        char('"'),
        map(take_until("\""), |s: Span| s.fragment().to_string()),
        char('"')
    ).parse(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("as")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, alias) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;

    Ok((input, Import {
        path,
        alias,
        location: start_location,
    }))
}

// Parse scoped identifier (namespace::name) or simple identifier
fn parse_scoped_identifier_string(input: Span) -> IResult<Span, String> {
    let (input, first_part) = parse_identifier(input)?;

    // Try to parse :: and second part
    if let Ok((input2, _)) = tag::<&str, Span, nom::error::Error<Span>>("::")(input) {
        if let Ok((input3, second_part)) = parse_identifier(input2) {
            return Ok((input3, format!("{}::{}", first_part, second_part)));
        }
    }

    // Just a simple identifier
    Ok((input, first_part))
}

fn parse_variable(input: Span) -> IResult<Span, Variable> {
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
fn parse_route(input: Span) -> IResult<Span, Route> {
    let (input, method) = parse_method(input)?;
    let (input, _) = multispace0(input)?;
    let (input, path) = take_till1(|c| c == ' ' || c == '\n')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline_ref(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Route {
        method,
        path: path.fragment().to_string(),
        pipeline
    }))
}

// GraphQL parsing
fn parse_graphql_schema(input: Span) -> IResult<Span, GraphQLSchema> {
    let (input, _) = tag("graphqlSchema")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, sdl) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, GraphQLSchema { sdl }))
}

fn parse_query_resolver(input: Span) -> IResult<Span, QueryResolver> {
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

fn parse_mutation_resolver(input: Span) -> IResult<Span, MutationResolver> {
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

fn parse_type_resolver(input: Span) -> IResult<Span, TypeResolver> {
    let (input, _) = tag("resolver")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, type_name) = parse_identifier(input)?;
    let (input, _) = char('.')(input)?;
    let (input, field_name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, pipeline) = parse_pipeline(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, TypeResolver { type_name, field_name, pipeline }))
}

// Test parsing
fn parse_when(input: Span) -> IResult<Span, When> {
    alt((
        map(
            (
                tag("calling"),
                multispace0,
                parse_method,
                multispace0,
                take_till1(|c| c == '\n')
            ),
            |(_, _, method, _, path)| When::CallingRoute { method, path: path.fragment().to_string() }
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

fn parse_condition(input: Span) -> IResult<Span, Condition> {
    let (input, _) = multispace0(input)?;
    let (input, condition_type_str) = alt((tag("then"), tag("and"))).parse(input)?;
    let condition_type = match condition_type_str.fragment() {
        &"then" => ConditionType::Then,
        &"and" => ConditionType::And,
        _ => unreachable!(),
    };
    let (input, _) = multispace0(input)?;
    let (input, field) = take_till1(|c| c == ' ')(input)?;
    let (input, _) = multispace0(input)?;

    // Check if this is a call assertion: "then call query users with ..."
    if field.fragment() == &"call" {
        // Parse call target: "query users" or "mutation createTodo"
        let (input, call_type) = alt((tag("query"), tag("mutation"))).parse(input)?;
        let (input, _) = nom::character::complete::multispace1(input)?;
        let (input, call_name) = parse_identifier(input)?;
        let call_target = format!("{}.{}", call_type.fragment(), call_name);
        let (input, _) = multispace0(input)?;

        // Parse comparison (typically "with" or "with arguments")
        let (input, comparison) = alt((
            tag("with arguments"),
            tag("with")
        )).parse(input)?;
        let (input, _) = multispace0(input)?;

        // Parse value (expected arguments)
        let (input, value) = alt((
            map(parse_multiline_string, |s| s.to_string()),
            map(
                delimited(char('"'), take_until("\""), char('"')),
                |s: Span| s.fragment().to_string()
            ),
            map(take_till1(|c| c == '\n'), |s: Span| s.fragment().to_string()),
        )).parse(input)?;

        return Ok((input, Condition {
            condition_type,
            field: "call".to_string(),
            header_name: None,
            jq_expr: None,
            comparison: comparison.fragment().to_string(),
            value,
            is_call_assertion: true,
            call_target: Some(call_target),
            selector: None,
            dom_assert: None,
        }));
    }

    // Check if field is "selector" - if so, parse CSS selector and DOM assertion
    if field.fragment() == &"selector" {
        let (input, selector_val) = alt((
            map(parse_multiline_string, |s| s.to_string()),
            map(
                delimited(char('"'), take_until("\""), char('"')),
                |s: Span| s.fragment().to_string()
            ),
        )).parse(input)?;
        let (input, _) = multispace0(input)?;

        // Parse operation: exists | does | text | count | attribute
        let (input, operation) = take_till1(|c| c == ' ' || c == '\n')(input)?;
        let (input, _) = multispace0(input)?;

        match operation.fragment() {
            &"exists" => {
                return Ok((input, Condition {
                    condition_type,
                    field: "selector".to_string(),
                    header_name: None,
                    jq_expr: None,
                    comparison: "exists".to_string(),
                    value: "true".to_string(),
                    is_call_assertion: false,
                    call_target: None,
                    selector: Some(selector_val),
                    dom_assert: Some(DomAssertType::Exists),
                }));
            }
            &"does" => {
                // Parse "does not exist"
                let (input, _) = tag("not")(input)?;
                let (input, _) = multispace0(input)?;
                let (input, _) = tag("exist")(input)?;
                return Ok((input, Condition {
                    condition_type,
                    field: "selector".to_string(),
                    header_name: None,
                    jq_expr: None,
                    comparison: "does_not_exist".to_string(),
                    value: "false".to_string(),
                    is_call_assertion: false,
                    call_target: None,
                    selector: Some(selector_val),
                    dom_assert: Some(DomAssertType::Exists),
                }));
            }
            &"text" => {
                // Parse comparison and value
                let (input, comparison) = take_till1(|c| c == ' ')(input)?;
                let (input, _) = multispace0(input)?;
                let (input, value) = alt((
                    map(parse_multiline_string, |s| s.to_string()),
                    map(
                        delimited(char('"'), take_until("\""), char('"')),
                        |s: Span| s.fragment().to_string()
                    ),
                    map(take_till1(|c| c == '\n'), |s: Span| s.fragment().to_string()),
                )).parse(input)?;
                return Ok((input, Condition {
                    condition_type,
                    field: "selector".to_string(),
                    header_name: None,
                    jq_expr: None,
                    comparison: comparison.fragment().to_string(),
                    value,
                    is_call_assertion: false,
                    call_target: None,
                    selector: Some(selector_val),
                    dom_assert: Some(DomAssertType::Text),
                }));
            }
            &"count" => {
                // Parse comparison (equals | is | is greater than | is less than)
                // Read until we hit a number
                let (input, comp_text) = take_till1(|c: char| c.is_ascii_digit())(input)?;
                let comparison = comp_text.fragment().trim().to_string();
                let (input, value) = take_till1(|c| c == '\n')(input)?;
                return Ok((input, Condition {
                    condition_type,
                    field: "selector".to_string(),
                    header_name: None,
                    jq_expr: None,
                    comparison,
                    value: value.fragment().trim().to_string(),
                    is_call_assertion: false,
                    call_target: None,
                    selector: Some(selector_val),
                    dom_assert: Some(DomAssertType::Count),
                }));
            }
            &"attribute" => {
                // Parse attribute name
                let (input, attr_name) = alt((
                    map(parse_multiline_string, |s| s.to_string()),
                    map(
                        delimited(char('"'), take_until("\""), char('"')),
                        |s: Span| s.fragment().to_string()
                    ),
                )).parse(input)?;
                let (input, _) = multispace0(input)?;
                // Parse comparison - read until we hit a quote or backtick (start of value)
                let (input, comparison) = take_till1(|c| c == '"' || c == '`')(input)?;
                let comparison = comparison.fragment().trim();
                let (input, value) = alt((
                    map(parse_multiline_string, |s| s.to_string()),
                    map(
                        delimited(char('"'), take_until("\""), char('"')),
                        |s: Span| s.fragment().to_string()
                    ),
                    map(take_till1(|c| c == '\n'), |s: Span| s.fragment().to_string()),
                )).parse(input)?;
                return Ok((input, Condition {
                    condition_type,
                    field: "selector".to_string(),
                    header_name: None,
                    jq_expr: None,
                    comparison: comparison.to_string(),
                    value,
                    is_call_assertion: false,
                    call_target: None,
                    selector: Some(selector_val),
                    dom_assert: Some(DomAssertType::Attribute(attr_name)),
                }));
            }
            _ => {
                return Err(nom::Err::Failure(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Tag
                )));
            }
        }
    }

    // Check if field is "header" - if so, parse header name
    let (input, header_name_opt) = if field.fragment() == &"header" {
        let (input, header_val) = alt((
            map(parse_multiline_string, |s| s.to_string()),
            map(
                delimited(char('"'), take_until("\""), char('"')),
                |s: Span| s.fragment().to_string()
            ),
        )).parse(input)?;
        let (input, _) = multispace0(input)?;
        (input, Some(header_val))
    } else {
        (input, None)
    };

    // Optional backticked jq filter following the field (not for header assertions)
    let (input, jq_expr_opt) = if header_name_opt.is_none() {
        let (input, jq) = opt(parse_multiline_string).parse(input)?;
        (input, jq)
    } else {
        (input, None)
    };
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
            |s: Span| s.fragment().to_string()
        ),
        // bare until end of line
        map(take_till1(|c| c == '\n'), |s: Span| s.fragment().to_string()),
    )).parse(input)?;
    Ok((input, Condition {
        condition_type,
        field: field.fragment().to_string(),
        header_name: header_name_opt,
        jq_expr: jq_expr_opt,
        comparison: comparison.fragment().to_string(),
        value,
        is_call_assertion: false,
        call_target: None,
        selector: None,
        dom_assert: None,
    }))
}

fn parse_mock(input: Span) -> IResult<Span, Mock> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("with")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("mock")(input)?;
    let (input, _) = multispace0(input)?;

    // Parse target - check for "pipeline", "query", "mutation" keywords or single identifier
    let (input, target) = if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("pipeline")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("pipeline.{}", name))
    } else if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("query")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("query.{}", name))
    } else if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("mutation")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("mutation.{}", name))
    } else {
        // Handle existing single identifiers like "pg" or "pg.var"
        let (input2, target) = take_till1(|c: char| c == ' ' || c == '\n')(input)?;
        (input2, target.fragment().to_string())
    };

    let (input, _) = multispace0(input)?;
    let (input, _) = tag("returning")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, return_value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Mock {
        target,
        return_value
    }))
}

fn parse_and_mock(input: Span) -> IResult<Span, Mock> {
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("and")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("mock")(input)?;
    let (input, _) = multispace0(input)?;

    // Parse target - check for "pipeline", "query", "mutation" keywords or single identifier
    let (input, target) = if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("pipeline")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("pipeline.{}", name))
    } else if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("query")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("query.{}", name))
    } else if let Ok((input2, _)) = tag::<_, _, nom::error::Error<_>>("mutation")(input) {
        let (input2, _) = nom::character::complete::multispace1(input2)?;
        let (input2, name) = parse_identifier(input2)?;
        (input2, format!("mutation.{}", name))
    } else {
        // Handle existing single identifiers like "pg" or "pg.var"
        let (input2, target) = take_till1(|c: char| c == ' ' || c == '\n')(input)?;
        (input2, target.fragment().to_string())
    };

    let (input, _) = multispace0(input)?;
    let (input, _) = tag("returning")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, return_value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, Mock {
        target,
        return_value
    }))
}

fn parse_let_binding(input: Span) -> IResult<Span, (String, String, LetValueFormat)> {
    let (input, _) = tag("let")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('=')(input)?;
    let (input, _) = multispace0(input)?;

    // Parse value: supports backtick strings, quoted strings, numbers (int/float), booleans, and null
    // Try each parser and track which format was used
    let (input, (value, format)) = alt((
        // Try backtick string first
        map(parse_multiline_string, |v| (v, LetValueFormat::Backtick)),
        // Try quoted string
        map(
            delimited(
                char('"'),
                map(take_until("\""), |s: Span| s.fragment().to_string()),
                char('"')
            ),
            |v| (v, LetValueFormat::Quoted)
        ),
        // Try null
        map(tag("null"), |s: Span| (s.fragment().to_string(), LetValueFormat::Bare)),
        // Try boolean
        map(alt((tag("true"), tag("false"))), |s: Span| (s.fragment().to_string(), LetValueFormat::Bare)),
        // Try float (must come before integer to match decimal point)
        map(
            recognize((
                digit1,
                char('.'),
                digit1,
            )),
            |s: Span| (s.fragment().to_string(), LetValueFormat::Bare)
        ),
        // Try integer
        map(digit1, |s: Span| (s.fragment().to_string(), LetValueFormat::Bare)),
    )).parse(input)?;

    Ok((input, (name, value, format)))
}

fn parse_with_clause(input: Span) -> IResult<Span, (String, String)> {
    let (input, _) = tag("with")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, kind) = alt((tag("input"), tag("body"), tag("headers"), tag("cookies"))).parse(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, (kind.fragment().to_string(), value)))
}

fn parse_and_with_clause(input: Span) -> IResult<Span, (String, String)> {
    let (input, _) = tag("and")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = tag("with")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, kind) = alt((tag("input"), tag("body"), tag("headers"), tag("cookies"))).parse(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = parse_multiline_string(input)?;
    let (input, _) = multispace0(input)?;
    Ok((input, (kind.fragment().to_string(), value)))
}

fn parse_it(input: Span) -> IResult<Span, It> {
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, _) = tag("it")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('"')(input)?;
    let (input, name) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // Parse optional per-test mocks
    let (input, mocks) = many0(parse_mock).parse(input)?;

    // Parse optional let bindings (before when clause)
    let mut variables: Vec<(String, String, LetValueFormat)> = Vec::new();
    let mut current_input = input;
    loop {
        if let Ok((new_input, (name, value, format))) = parse_let_binding(current_input) {
            variables.push((name, value, format));
            let (new_input, _) = skip_ws_and_comments(new_input)?;
            current_input = new_input;
        } else {
            break;
        }
    }

    let (input, _) = tag("when")(current_input)?;
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, when) = parse_when(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // Parse optional with clauses
    let mut input_opt = None;
    let mut body_opt = None;
    let mut headers_opt = None;
    let mut cookies_opt = None;
    let mut current_input = input;
    let mut first_with = true;

    loop {
        // Try parsing a with clause
        let with_result = if first_with {
            opt(parse_with_clause).parse(current_input)
        } else {
            opt(parse_and_with_clause).parse(current_input)
        };

        match with_result {
            Ok((new_input, Some((kind, value)))) => {
                match kind.as_str() {
                    "input" => input_opt = Some(value),
                    "body" => body_opt = Some(value),
                    "headers" => headers_opt = Some(value),
                    "cookies" => cookies_opt = Some(value),
                    _ => {}
                }
                current_input = new_input;
                first_with = false;
            }
            Ok((_, None)) => break,
            Err(_) => break,
        }
    }

    let (input, _) = skip_ws_and_comments(current_input)?;

    // Parse optional additional mocks starting with 'and mock'
    let (input, extra_mocks) = many0(parse_and_mock).parse(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // Parse remaining conditions
    let (input, conditions) = many0(parse_condition).parse(input)?;

    let mut all_mocks = mocks;
    all_mocks.extend(extra_mocks);

    Ok((input, It {
        name: name.fragment().to_string(),
        mocks: all_mocks,
        when,
        variables,
        input: input_opt,
        body: body_opt,
        headers: headers_opt,
        cookies: cookies_opt,
        conditions
    }))
}

fn parse_describe(input: Span) -> IResult<Span, Describe> {
    let (input, _) = skip_ws_and_comments(input)?;
    let (input, _) = tag("describe")(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = char('"')(input)?;
    let (input, name) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    let (input, _) = skip_ws_and_comments(input)?;

    // Parse let bindings, mocks, and tests in any order
    let mut variables = Vec::new();
    let mut mocks = Vec::new();
    let mut tests = Vec::new();
    let mut remaining = input;

    loop {
        let (new_remaining, _) = skip_ws_and_comments(remaining)?;

        // Try to parse a let binding
        if let Ok((new_input, binding)) = parse_let_binding(new_remaining) {
            variables.push(binding);
            remaining = new_input;
            continue;
        }

        // Try to parse a mock (with mock or and mock)
        if let Ok((new_input, mock)) = parse_mock(new_remaining) {
            mocks.push(mock);
            remaining = new_input;
            continue;
        }

        if let Ok((new_input, mock)) = parse_and_mock(new_remaining) {
            mocks.push(mock);
            remaining = new_input;
            continue;
        }

        // Try to parse an it block
        if let Ok((new_input, test)) = parse_it(new_remaining) {
            tests.push(test);
            remaining = new_input;
            continue;
        }

        // Nothing more to parse
        break;
    }

    Ok((remaining, Describe {
        name: name.fragment().to_string(),
        variables,
        mocks,
        tests
    }))
}

// Top-level program parsing
pub fn parse_program(input: &str) -> IResult<&str, Program> {
    // Wrap the input string in a Span to track location
    let span_input = Span::new(input);
    let (span_remaining, _) = multispace0::<Span, nom::error::Error<Span>>(span_input)
        .map_err(|e| e.map_input(|s| *s.fragment()))?;

    let mut configs = Vec::new();
    let mut imports = Vec::new();
    let mut pipelines = Vec::new();
    let mut variables = Vec::new();
    let mut routes = Vec::new();
    let mut describes = Vec::new();
    let mut graphql_schema = None;
    let mut queries = Vec::new();
    let mut mutations = Vec::new();
    let mut resolvers = Vec::new();
    let mut feature_flags = None;

    let mut remaining = span_remaining;

    while !remaining.fragment().is_empty() {
        let (new_remaining, _) = multispace0::<Span, nom::error::Error<Span>>(remaining)
            .map_err(|e| e.map_input(|s| *s.fragment()))?;
        if new_remaining.fragment().is_empty() {
            break;
        }

        // Try parsing each type of top-level element
        if let Ok((new_input, config)) = parse_config(new_remaining) {
            configs.push(config);
            remaining = new_input;
        } else if let Ok((new_input, import)) = parse_import(new_remaining) {
            // Check for duplicate aliases
            if imports.iter().any(|i: &Import| i.alias == import.alias) {
                // Duplicate alias found - return error
                return Err(nom::Err::Failure(nom::error::Error::new(
                    *new_input.fragment(),
                    nom::error::ErrorKind::Verify
                )));
            }
            imports.push(import);
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
        } else if let Ok((new_input, resolver)) = parse_type_resolver(new_remaining) {
            resolvers.push(resolver);
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
            if let Ok((new_input, _)) = take_till1::<_, _, nom::error::Error<_>>(|c| c == '\n').parse(new_remaining) {
                remaining = new_input;
            } else {
                break;
            }
        }
    }

    // Convert the Span back to &str for the IResult
    Ok((remaining.fragment(), Program { configs, imports, pipelines, variables, routes, describes, graphql_schema, queries, mutations, resolvers, feature_flags }))
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

        // Step 0: @prod (single tag)
        if let PipelineStep::Regular { condition: Some(TagExpr::Tag(tag)), .. } = &pipeline.steps[0] {
            assert_eq!(tag.name, "prod");
            assert_eq!(tag.negated, false);
            assert_eq!(tag.args.len(), 0);
        } else {
            panic!("Expected Regular step with single tag");
        }

        // Step 1: @dev @flag(new-ui) (implicit AND of two tags)
        if let PipelineStep::Regular { condition: Some(expr), .. } = &pipeline.steps[1] {
            // Should be: And(Tag(dev), Tag(flag(new-ui)))
            if let TagExpr::And(left, right) = expr {
                if let TagExpr::Tag(dev_tag) = left.as_ref() {
                    assert_eq!(dev_tag.name, "dev");
                } else {
                    panic!("Expected dev tag on left");
                }
                if let TagExpr::Tag(flag_tag) = right.as_ref() {
                    assert_eq!(flag_tag.name, "flag");
                    assert_eq!(flag_tag.args, vec!["new-ui"]);
                } else {
                    panic!("Expected flag tag on right");
                }
            } else {
                panic!("Expected And expression");
            }
        } else {
            panic!("Expected Regular step with condition");
        }

        // Step 2: @!prod @async(user) (negated tag AND async tag)
        if let PipelineStep::Regular { condition: Some(expr), .. } = &pipeline.steps[2] {
            if let TagExpr::And(left, right) = expr {
                if let TagExpr::Tag(prod_tag) = left.as_ref() {
                    assert_eq!(prod_tag.name, "prod");
                    assert_eq!(prod_tag.negated, true);
                } else {
                    panic!("Expected prod tag on left");
                }
                if let TagExpr::Tag(async_tag) = right.as_ref() {
                    assert_eq!(async_tag.name, "async");
                    assert_eq!(async_tag.args, vec!["user"]);
                } else {
                    panic!("Expected async tag on right");
                }
            } else {
                panic!("Expected And expression");
            }
        } else {
            panic!("Expected Regular step with condition");
        }

        // Step 3: @flag(beta,staff) (single tag with multiple args)
        if let PipelineStep::Regular { condition: Some(TagExpr::Tag(tag)), .. } = &pipeline.steps[3] {
            assert_eq!(tag.name, "flag");
            assert_eq!(tag.args, vec!["beta", "staff"]);
        } else {
            panic!("Expected Regular step with single tag");
        }

        // Test roundtrip
        let displayed = format!("{}", pipeline);
        assert!(displayed.contains("@prod"));
        // Note: Display now uses TagExpr::Display which shows "X and Y" for And expressions
        assert!(displayed.contains("@dev and @flag(new-ui)"));
        assert!(displayed.contains("@!prod and @async(user)"));
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

        // Steps without tags should have condition = None
        if let PipelineStep::Regular { condition, .. } = &pipeline.steps[0] {
            assert!(condition.is_none());
        }
        if let PipelineStep::Regular { condition, .. } = &pipeline.steps[1] {
            assert!(condition.is_none());
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

        // Multiple space-separated tags are now parsed as implicit AND
        assert!(formatted.contains("@prod and @dev"));
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
            if let PipelineStep::If { condition, then_branch, else_branch, location: _ } = &pipeline.steps[1] {
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

    #[test]
    fn parse_dispatch_basic() {
        let src = r#"
GET /dashboard
  |> dispatch
    case @flag(experimental):
      |> pipeline: experimentalDash
    case @env(dev):
      |> pipeline: devDash
    default:
      |> pipeline: standardDash
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.routes.len(), 1);

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            assert_eq!(pipeline.steps.len(), 1);

            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                assert_eq!(branches.len(), 2);

                // Check first case: @flag(experimental)
                if let TagExpr::Tag(tag) = &branches[0].condition {
                    assert_eq!(tag.name, "flag");
                    assert_eq!(tag.args, vec!["experimental"]);
                } else {
                    panic!("Expected Tag");
                }
                assert_eq!(branches[0].pipeline.steps.len(), 1);

                // Check second case: @env(dev)
                if let TagExpr::Tag(tag) = &branches[1].condition {
                    assert_eq!(tag.name, "env");
                    assert_eq!(tag.args, vec!["dev"]);
                } else {
                    panic!("Expected Tag");
                }
                assert_eq!(branches[1].pipeline.steps.len(), 1);

                // Check default branch exists
                assert!(default.is_some());
                assert_eq!(default.as_ref().unwrap().steps.len(), 1);
            } else {
                panic!("Expected Dispatch step");
            }
        } else {
            panic!("Expected inline pipeline");
        }
    }

    #[test]
    fn parse_dispatch_no_default() {
        let src = r#"
GET /test
  |> dispatch
    case @flag(beta):
      |> jq: `{ version: "beta" }`
    case @flag(alpha):
      |> jq: `{ version: "alpha" }`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                assert_eq!(branches.len(), 2);
                assert!(default.is_none());
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }

    #[test]
    fn parse_dispatch_with_end_keyword() {
        let src = r#"
GET /test
  |> dispatch
    case @env(prod):
      |> jq: `{ env: "production" }`
    default:
      |> jq: `{ env: "other" }`
  end
  |> handlebars: `<p>{{env}}</p>`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            assert_eq!(pipeline.steps.len(), 2); // dispatch + handlebars

            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                assert_eq!(branches.len(), 1);
                assert!(default.is_some());
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }

    #[test]
    fn parse_dispatch_multi_step_pipelines() {
        let src = r#"
GET /complex
  |> dispatch
    case @flag(experimental):
      |> jq: `{ step: 1 }`
      |> pg: `SELECT * FROM experimental`
      |> handlebars: `<p>Experimental</p>`
    default:
      |> jq: `{ step: 1 }`
      |> handlebars: `<p>Default</p>`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                // First branch has 3 steps
                assert_eq!(branches[0].pipeline.steps.len(), 3);
                // Default branch has 2 steps
                assert_eq!(default.as_ref().unwrap().steps.len(), 2);
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }

    #[test]
    fn parse_dispatch_negated_tag() {
        let src = r#"
GET /test
  |> dispatch
    case @!env(production):
      |> jq: `{ mode: "non-production" }`
    default:
      |> jq: `{ mode: "production" }`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            if let PipelineStep::Dispatch { branches, .. } = &pipeline.steps[0] {
                if let TagExpr::Tag(tag) = &branches[0].condition {
                    assert_eq!(tag.name, "env");
                    assert_eq!(tag.negated, true);
                    assert_eq!(tag.args, vec!["production"]);
                } else {
                    panic!("Expected Tag");
                }
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }

    #[test]
    fn parse_dispatch_with_comments() {
        let src = r#"
GET /test
  # comment before dispatch
  |> dispatch
    # comment before first case
    case @flag(experimental):
      # comment inside branch
      |> jq: `{ version: "experimental" }`
    # comment between cases
    case @env(dev):
      |> jq: `{ env: "dev" }`
    # comment before default
    default:
      # comment inside default
      |> jq: `{ fallback: true }`
  # comment after dispatch
"#;
        let (_rest, program) = parse_program(src).unwrap();
        assert_eq!(program.routes.len(), 1);

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                assert_eq!(branches.len(), 2);
                assert!(default.is_some());
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }

    #[test]
    fn roundtrip_dispatch_formatting() {
        let src = r#"
pipeline test =
  |> dispatch
    case @flag(experimental):
      |> pipeline: experimentalDash
    case @env(dev):
      |> pipeline: devDash
    default:
      |> pipeline: standardDash
"#;
        let (_rest, program) = parse_program(src).unwrap();
        let formatted = format!("{}", program);

        // Check key elements are preserved
        assert!(formatted.contains("dispatch"));
        assert!(formatted.contains("case @flag(experimental):"));
        assert!(formatted.contains("case @env(dev):"));
        assert!(formatted.contains("default:"));
    }

    #[test]
    fn parse_dispatch_oneliner_syntax() {
        // Test that one-liner syntax works (case keyword is the delimiter)
        let src = r#"
GET /dash
  |> dispatch case @flag(exp): |> pipeline: exp case @env(dev): |> pipeline: dev default: |> pipeline: std end
"#;
        let (_rest, program) = parse_program(src).unwrap();

        if let PipelineRef::Inline(pipeline) = &program.routes[0].pipeline {
            if let PipelineStep::Dispatch { branches, default, location: _ } = &pipeline.steps[0] {
                assert_eq!(branches.len(), 2);
                assert!(default.is_some());
                if let TagExpr::Tag(tag0) = &branches[0].condition {
                    assert_eq!(tag0.name, "flag");
                } else {
                    panic!("Expected Tag");
                }
                if let TagExpr::Tag(tag1) = &branches[1].condition {
                    assert_eq!(tag1.name, "env");
                } else {
                    panic!("Expected Tag");
                }
            } else {
                panic!("Expected Dispatch step");
            }
        }
    }
}
