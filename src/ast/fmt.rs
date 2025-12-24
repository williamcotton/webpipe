use std::fmt::Display;

use super::types::*;

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
        for resolver in &self.resolvers {
            writeln!(f, "{}", resolver)?;
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
            // Format arguments - wrap in backticks if they contain special characters or spaces
            let formatted_args: Vec<String> = self.args.iter().map(|arg| {
                // If arg contains special characters, wrap in backticks
                if arg.contains(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
                    format!("`{}`", arg)
                } else {
                    arg.clone()
                }
            }).collect();
            write!(f, "({})", formatted_args.join(","))?;
        }
        Ok(())
    }
}

impl Display for TagExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagExpr::Tag(tag) => write!(f, "{}", tag),
            TagExpr::And(left, right) => {
                // Add parentheses around OR expressions inside AND for clarity
                let left_str = match left.as_ref() {
                    TagExpr::Or(_, _) => format!("({})", left),
                    _ => format!("{}", left),
                };
                let right_str = match right.as_ref() {
                    TagExpr::Or(_, _) => format!("({})", right),
                    _ => format!("{}", right),
                };
                write!(f, "{} and {}", left_str, right_str)
            }
            TagExpr::Or(left, right) => write!(f, "{} or {}", left, right),
        }
    }
}

impl Display for PipelineStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStep::Regular { name, args, config, config_type, condition, .. } => {
                // Format arguments if present
                let args_str = if !args.is_empty() {
                    format!("({})", args.join(", "))
                } else {
                    String::new()
                };

                let formatted_config = match config_type {
                    ConfigType::Backtick => format!("`{}`", config),
                    ConfigType::Quoted => format!("\"{}\"", config),
                    ConfigType::Identifier => config.clone(),
                };
                write!(f, "  |> {}{}: {}", name, args_str, formatted_config)?;

                // Append condition if present
                if let Some(cond) = condition {
                    write!(f, " {}", cond)?;
                }
                Ok(())
            }
            PipelineStep::Result { branches, .. } => {
                writeln!(f, "  |> result")?;
                for branch in branches {
                    write!(f, "    {}", branch)?;
                }
                Ok(())
            }
            PipelineStep::If { condition, then_branch, else_branch, .. } => {
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
            PipelineStep::Dispatch { branches, default, .. } => {
                writeln!(f, "  |> dispatch")?;
                // Format case branches
                for branch in branches {
                    write!(f, "    case {}: ", branch.condition)?;
                    // If pipeline has only one step, format inline
                    if branch.pipeline.steps.len() == 1 {
                        writeln!(f, "{}", branch.pipeline.steps[0])?;
                    } else {
                        writeln!(f)?;
                        for step in &branch.pipeline.steps {
                            writeln!(f, "    {}", step)?;
                        }
                    }
                }
                // Format default branch if present
                if let Some(default_pipe) = default {
                    writeln!(f, "    default:")?;
                    for step in &default_pipe.steps {
                        writeln!(f, "    {}", step)?;
                    }
                }
                Ok(())
            }
            PipelineStep::Foreach { selector, pipeline, .. } => {
                writeln!(f, "  |> foreach {}", selector)?;
                // Format inner pipeline with proper indentation
                for step in &pipeline.steps {
                    writeln!(f, "  {}", step)?;
                }
                write!(f, "  end")
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

        // Print let bindings
        for (name, value, format) in &self.variables {
            let formatted_value = match format {
                LetValueFormat::Quoted => format!("\"{}\"", value),
                LetValueFormat::Backtick => format!("`{}`", value),
                LetValueFormat::Bare => value.clone(),
            };
            writeln!(f, "    let {} = {}", name, formatted_value)?;
        }

        if let Some(input) = &self.input {
            writeln!(f, "    with input `{}`", input)?;
        }
        if let Some(body) = &self.body {
            writeln!(f, "    with body `{}`", body)?;
        }
        if let Some(headers) = &self.headers {
            writeln!(f, "    with headers `{}`", headers)?;
        }
        if let Some(cookies) = &self.cookies {
            writeln!(f, "    with cookies `{}`", cookies)?;
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
        // Handle selector assertions
        if let Some(selector) = &self.selector {
            if let Some(dom_assert) = &self.dom_assert {
                return match dom_assert {
                    DomAssertType::Exists => {
                        let operation = if self.comparison == "exists" {
                            "exists"
                        } else {
                            "does not exist"
                        };
                        write!(f, "{} selector \"{}\" {}", self.condition_type, selector, operation)
                    }
                    DomAssertType::Text => {
                        write!(f, "{} selector \"{}\" text {} {}", self.condition_type, selector, self.comparison, self.value)
                    }
                    DomAssertType::Count => {
                        write!(f, "{} selector \"{}\" count {} {}", self.condition_type, selector, self.comparison, self.value)
                    }
                    DomAssertType::Attribute(attr_name) => {
                        write!(f, "{} selector \"{}\" attribute \"{}\" {} {}",
                            self.condition_type, selector, attr_name, self.comparison, self.value)
                    }
                };
            }
        }

        if self.is_call_assertion {
            // Format: "then call query users with `{ "id": 1 }`"
            if let Some(target) = &self.call_target {
                write!(f, "{} call {} {} {}", self.condition_type, target.replace('.', " "), self.comparison, self.value)
            } else {
                write!(f, "{} call {} {}", self.condition_type, self.comparison, self.value)
            }
        } else {
            let field_part = if let Some(header) = &self.header_name {
                format!("{} \"{}\"", self.field, header)
            } else if let Some(expr) = &self.jq_expr {
                format!("{} `{}`", self.field, expr)
            } else {
                self.field.clone()
            };
            write!(f, "{} {} {} {}", self.condition_type, field_part, self.comparison, self.value)
        }
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

impl Display for TypeResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "resolver {}.{} =", self.type_name, self.field_name)?;
        write!(f, "{}", self.pipeline)
    }
}

