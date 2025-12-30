/// Source location information for AST nodes
/// Tracks line and column position in the original source file
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Byte offset from start of file
    pub offset: usize,
    /// Source file path (for multi-file debugging and error reporting)
    pub file_path: Option<String>,
}

impl SourceLocation {
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self { line, column, offset, file_path: None }
    }

    pub fn with_file(line: usize, column: usize, offset: usize, file_path: String) -> Self {
        Self { line, column, offset, file_path: Some(file_path) }
    }
}

/// Import statement for file-based modules
/// Syntax: import "./path/to/file.wp" as alias
#[derive(Debug, Clone)]
pub struct Import {
    /// Import path (e.g., "./lib/auth.wp")
    pub path: String,
    /// Import alias (e.g., "auth")
    pub alias: String,
    /// Source location for error reporting
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub configs: Vec<Config>,
    pub imports: Vec<Import>,
    pub pipelines: Vec<NamedPipeline>,
    pub variables: Vec<Variable>,
    pub routes: Vec<Route>,
    pub describes: Vec<Describe>,
    pub graphql_schema: Option<GraphQLSchema>,
    pub queries: Vec<QueryResolver>,
    pub mutations: Vec<MutationResolver>,
    pub resolvers: Vec<TypeResolver>,
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
pub struct TypeResolver {
    pub type_name: String,
    pub field_name: String,
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
pub enum LetValueFormat {
    Quoted,
    Backtick,
    Bare,
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

/// A boolean expression of tags for dispatch routing
/// Supports AND, OR operations with standard precedence (AND binds tighter than OR)
#[derive(Debug, Clone)]
pub enum TagExpr {
    /// A single tag: @when(admin), @!flag(beta), @env(prod)
    Tag(Tag),
    
    /// Logical AND: expr1 and expr2
    And(Box<TagExpr>, Box<TagExpr>),
    
    /// Logical OR: expr1 or expr2
    Or(Box<TagExpr>, Box<TagExpr>),
}

#[derive(Debug, Clone)]
pub enum PipelineStep {
    Regular {
        name: String,
        /// Inline arguments for the middleware (e.g., fetch(url, options) -> ["url", "options"])
        /// These are raw JQ expression strings that will be evaluated at runtime
        args: Vec<String>,
        config: String,
        config_type: ConfigType,
        /// Optional tag expression for conditional execution
        /// Supports boolean expressions: @when(a) and @when(b), @flag(x) or @env(dev)
        condition: Option<TagExpr>,
        /// Pre-parsed join task names (only populated for "join" middleware)
        /// This avoids parsing the config string on every request in the hot path.
        parsed_join_targets: Option<Vec<String>>,
        /// Source location for debugging and error reporting
        location: SourceLocation,
    },
    Result {
        branches: Vec<ResultBranch>,
        /// Source location for debugging and error reporting
        location: SourceLocation,
    },
    If {
        condition: Pipeline,
        then_branch: Pipeline,
        else_branch: Option<Pipeline>,
        /// Source location for debugging and error reporting
        location: SourceLocation,
    },
    Dispatch {
        branches: Vec<DispatchBranch>,
        default: Option<Pipeline>,
        /// Source location for debugging and error reporting
        location: SourceLocation,
    },
    /// A structural block that iterates over a JSON array
    /// Syntax: |> foreach data.rows
    Foreach {
        /// The selector string, e.g., "data.rows"
        selector: String,
        /// The inner pipeline to execute for each item
        pipeline: Pipeline,
        /// Source location for debugging and error reporting
        location: SourceLocation,
    },
}

impl PipelineStep {
    /// Get the source location of this step
    /// Used by the debugger to match breakpoints
    pub fn location(&self) -> &SourceLocation {
        match self {
            PipelineStep::Regular { location, .. } => location,
            PipelineStep::Result { location, .. } => location,
            PipelineStep::If { location, .. } => location,
            PipelineStep::Dispatch { location, .. } => location,
            PipelineStep::Foreach { location, .. } => location,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DispatchBranch {
    pub condition: TagExpr,
    pub pipeline: Pipeline,
    /// Source location for debugging and error reporting
    pub location: SourceLocation,
}

#[derive(Debug, Clone)]
pub struct ResultBranch {
    pub branch_type: ResultBranchType,
    pub status_code: u16,
    pub pipeline: Pipeline,
    /// Source location for debugging and error reporting
    pub location: SourceLocation,
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
    pub variables: Vec<(String, String, LetValueFormat)>,
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
    pub variables: Vec<(String, String, LetValueFormat)>,
    pub input: Option<String>,
    pub body: Option<String>,
    pub headers: Option<String>,
    pub cookies: Option<String>,
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
    pub header_name: Option<String>, // For "header Set-Cookie contains ..."
    pub jq_expr: Option<String>,
    pub comparison: String,
    pub value: String,
    /// True if this is a call verification (e.g., "then call query users with ...")
    pub is_call_assertion: bool,
    /// Target for call assertions (e.g., "query.users")
    pub call_target: Option<String>,
    /// CSS selector for DOM assertions (e.g., "h1.title", "#login")
    pub selector: Option<String>,
    /// Type of DOM assertion being performed
    pub dom_assert: Option<DomAssertType>,
}

#[derive(Debug, Clone)]
pub enum ConditionType {
    Then,
    And,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DomAssertType {
    /// Check if element(s) exist
    Exists,
    /// Check element inner text
    Text,
    /// Check count of matched elements
    Count,
    /// Check a specific attribute value (e.g., "href")
    Attribute(String),
}
