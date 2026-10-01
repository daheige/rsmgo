//! Language → spec table and per-family keyword sets for the highlighter.
//!
//! Every alias (100+ languages, from `c`/`cpp`/`java`/`go`/`rust`/`js`/`ts`/
//! `py`/`rb` to `sol`/`zig`/`nim`/`gdscript`/`cobol`/`typst`/...) resolves to
//! a [`LangSpec`] holding its comment syntax and keyword set. Keywords are
//! curated per language family (C-like, JVM, JS/TS, Rust, Go, Python, shell,
//! SQL, ML, Lisp, ...), so a family union may highlight a sibling's keyword —
//! an accepted trade-off for a dependency-free highlighter. JSON and other
//! data formats get strings/numbers/literals with no comment syntax at all.

/// How comments look in a given language.
#[derive(Clone, Copy, PartialEq)]
pub enum CommentStyle {
    /// No comments (JSON, HTML, COBOL, batch, ...).
    None,
    /// `//` line comments and `/* ... */` blocks.
    CStyle,
    /// `#` to end of line.
    Hash,
    /// `--` to end of line (SQL, Lua, Haskell, Elm, ...).
    Dash,
    /// `%` to end of line (Erlang, MATLAB, TeX, Prolog, ...).
    Percent,
    /// `;` to end of line (Lisp, Clojure, Scheme, assembly, INI, ...).
    Semicolon,
    /// `!` to end of line (Fortran).
    Bang,
    /// `'` to end of line (VB / VBA; string scanning runs first, so
    /// apostrophes inside `"..."` strings are safe).
    Apostrophe,
    /// `(* ... *)` blocks (OCaml, Pascal without `//`).
    ParenStar,
}

/// Per-language highlighting behavior.
pub struct LangSpec {
    pub comments: CommentStyle,
    pub keywords: &'static [&'static str],
    /// Match keywords case-insensitively (SQL is typically written uppercase).
    pub ignore_case: bool,
}

const fn spec(comments: CommentStyle, keywords: &'static [&'static str]) -> LangSpec {
    LangSpec {
        comments,
        keywords,
        ignore_case: false,
    }
}

/// Fallback for unknown languages: C-style comments, no keywords.
pub const GENERIC: LangSpec = spec(CommentStyle::CStyle, &[]);

/// Data formats and markup: no comments, no keywords (strings/numbers/
/// literals are still colored by the scanner; JSON keys are strings).
pub const PLAIN: LangSpec = spec(CommentStyle::None, &[]);
/// JSON with comments (`//`, `/* */`), e.g. tsconfig.json.
pub const JSONC: LangSpec = spec(CommentStyle::CStyle, &[]);

// ---------------------------------------------------------------------------
// Keyword sets (one per language family)
// ---------------------------------------------------------------------------

pub const KW_CSS: &[&str] = &[
    "media", "supports", "keyframes", "import", "font-face", "page", "charset", "namespace",
    "layer", "container", "scope", "root", "hover", "focus", "active", "visited", "link",
    "before", "after", "selection", "placeholder", "marker", "backdrop", "host", "slotted",
];

pub const KW_C: &[&str] = &[
    "alignas", "alignof", "and", "and_eq", "asm", "auto", "bitand", "bitor", "bool", "break",
    "case", "catch", "char", "char8_t", "char16_t", "char32_t", "class", "compl", "concept",
    "const", "consteval", "constexpr", "constinit", "const_cast", "continue", "co_await",
    "co_return", "co_yield", "decltype", "default", "delete", "do", "double", "dynamic_cast",
    "else", "enum", "explicit", "export", "extern", "false", "final", "float", "for", "friend",
    "goto", "if", "implements", "import", "inline", "instanceof", "int", "interface", "long",
    "module", "mutable", "namespace", "new", "noexcept", "not", "not_eq", "nullptr", "operator",
    "or", "or_eq", "override", "package", "private", "protected", "public", "register",
    "reinterpret_cast", "requires", "restrict", "return", "short", "signed", "sizeof", "static",
    "static_assert", "static_cast", "struct", "super", "switch", "synchronized", "template",
    "this", "throw", "true", "try", "typedef", "typeid", "typename", "typeof", "union",
    "unsigned", "using", "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
    "NULL", "nil", "YES", "NO", "self", "id", "SEL", "instancetype", "nonatomic", "strong",
    "weak", "copy", "assign", "retain", "readonly", "nullable", "nonnull",
];

pub const KW_JVM: &[&str] = &[
    "abstract", "actual", "annotation", "as", "assert", "by", "byte", "case", "catch", "char",
    "class", "companion", "const", "constructor", "continue", "crossinline", "data", "def",
    "default", "delegate", "do", "double", "else", "enum", "event", "expect", "extends",
    "external", "field", "file", "final", "finally", "float", "for", "fun", "get", "goto",
    "if", "implements", "implicit", "import", "in", "init", "inline", "inner", "instanceof",
    "int", "interface", "internal", "is", "it", "lazy", "let", "long", "noinline", "null",
    "object", "open", "operator", "out", "override", "package", "param", "private", "property",
    "protected", "public", "receiver", "record", "reified", "return", "sealed", "set",
    "setparam", "short", "static", "strictfp", "super", "suspend", "switch", "synchronized",
    "tailrec", "this", "throw", "throws", "transient", "try", "typealias", "val", "var",
    "vararg", "void", "volatile", "when", "while", "with", "yield", "true", "false",
];

pub const KW_JS: &[&str] = &[
    "abstract", "accessor", "any", "as", "assert", "asserts", "async", "await", "bigint",
    "boolean", "break", "case", "catch", "class", "const", "constructor", "continue", "debugger",
    "declare", "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally",
    "for", "from", "function", "get", "if", "implements", "import", "in", "infer", "instanceof",
    "interface", "is", "keyof", "let", "namespace", "never", "new", "null", "number", "object",
    "of", "out", "public", "private", "protected", "readonly", "require", "return", "satisfies",
    "set", "static", "string", "super", "switch", "symbol", "this", "throw", "true", "try",
    "type", "typeof", "undefined", "unique", "unknown", "var", "void", "while", "with", "yield",
    "console", "process", "window", "document", "globalThis",
];

pub const KW_RUST: &[&str] = &[
    "Self", "as", "async", "await", "box", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "macro_rules",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super",
    "trait", "true", "type", "union", "unsafe", "use", "where", "while",
];

pub const KW_GO: &[&str] = &[
    "any", "append", "bool", "break", "byte", "cap", "case", "chan", "close", "complex128",
    "complex64", "const", "continue", "copy", "default", "defer", "delete", "else", "error",
    "fallthrough", "false", "float32", "float64", "for", "func", "go", "goto", "if", "import",
    "int", "int16", "int32", "int64", "int8", "interface", "iota", "len", "make", "map", "new",
    "nil", "package", "panic", "print", "println", "range", "recover", "return", "rune",
    "select", "string", "struct", "switch", "true", "type", "uint", "uint16", "uint32", "uint64",
    "uint8", "uintptr", "var",
];

pub const KW_SWIFT: &[&str] = &[
    "Any", "Self", "actor", "as", "associatedtype", "async", "await", "break", "case", "catch",
    "class", "continue", "default", "defer", "deinit", "do", "else", "enum", "extension",
    "fallthrough", "false", "fileprivate", "for", "func", "guard", "if", "import", "in", "init",
    "inout", "internal", "is", "let", "nil", "open", "operator", "precedencegroup", "private",
    "protocol", "public", "repeat", "rethrows", "return", "self", "some", "static", "struct",
    "subscript", "super", "switch", "throw", "throws", "true", "try", "typealias", "var",
    "where", "while",
];

pub const KW_PHP: &[&str] = &[
    "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone",
    "const", "continue", "declare", "default", "do", "echo", "else", "elseif", "empty",
    "enddeclare", "endfor", "endforeach", "endif", "endswitch", "endwhile", "enum", "eval",
    "exit", "extends", "final", "finally", "fn", "for", "foreach", "from", "function", "global",
    "goto", "if", "implements", "include", "include_once", "instanceof", "insteadof", "interface",
    "int", "isset", "iterable", "list", "match", "mixed", "namespace", "never", "new", "null",
    "object", "or", "parent", "print", "private", "protected", "public", "readonly", "require",
    "require_once", "return", "self", "static", "string", "switch", "throw", "trait", "true",
    "try", "unset", "use", "var", "void", "while", "xor", "yield",
];

pub const KW_PYTHON: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "bool", "break", "class",
    "cls", "continue", "def", "del", "dict", "elif", "else", "except", "finally", "float", "for",
    "from", "global", "if", "import", "in", "input", "int", "is", "lambda", "len", "list", "nonlocal",
    "not", "or", "pass", "print", "raise", "range", "return", "self", "set", "str", "tuple",
    "try", "while", "with", "yield",
];

pub const KW_SHELL: &[&str] = &[
    "alias", "bg", "break", "builtin", "case", "cd", "command", "continue", "coproc", "declare",
    "do", "done", "echo", "elif", "else", "enable", "esac", "eval", "exec", "exit", "export",
    "false", "fi", "for", "function", "getopts", "help", "if", "in", "jobs", "kill", "local",
    "logout", "printf", "pwd", "read", "readonly", "return", "select", "set", "shift", "shopt",
    "source", "suspend", "test", "then", "time", "trap", "true", "type", "typeset", "ulimit",
    "umask", "unalias", "unset", "until", "wait", "while",
];

pub const KW_SQL: &[&str] = &[
    "all", "alter", "and", "any", "as", "asc", "avg", "begin", "between", "by", "case", "cast",
    "check", "coalesce", "collate", "column", "commit", "constraint", "count", "create", "cross",
    "current_date", "current_time", "current_timestamp", "database", "default", "delete", "desc",
    "distinct", "drop", "else", "end", "escape", "except", "exists", "explain", "false",
    "foreign", "from", "full", "group", "having", "if", "in", "index", "inner", "insert",
    "intersect", "into", "is", "join", "key", "lag", "lead", "left", "like", "limit", "max",
    "min", "not", "null", "offset", "on", "or", "order", "outer", "over", "partition", "primary",
    "references", "rename", "replace", "right", "rollback", "row_number", "schema", "select",
    "set", "sum", "table", "then", "to", "transaction", "trigger", "true", "union", "unique",
    "update", "using", "values", "view", "when", "where", "window", "with",
];

pub const KW_RUBY: &[&str] = &[
    "BEGIN", "END", "alias", "and", "attr_accessor", "attr_reader", "attr_writer", "begin",
    "break", "case", "catch", "class", "def", "defined", "do", "each", "else", "elsif", "end",
    "ensure", "extend", "fail", "false", "for", "freeze", "if", "include", "in", "lambda",
    "loop", "module", "new", "next", "nil", "not", "or", "prepend", "puts", "raise", "redo",
    "require", "require_relative", "rescue", "retry", "return", "self", "super", "then", "throw",
    "true", "undef", "unless", "until", "when", "while", "yield",
];

pub const KW_PERL: &[&str] = &[
    "and", "bless", "break", "catch", "chomp", "chop", "cmp", "confess", "continue", "croak",
    "defined", "delete", "die", "do", "each", "else", "elsif", "eq", "eval", "exists", "for",
    "foreach", "ge", "given", "gt", "if", "last", "le", "local", "lt", "my", "ne", "next", "no",
    "not", "or", "our", "package", "print", "redo", "ref", "require", "return", "say", "state",
    "sub", "switch", "then", "undef", "unless", "until", "use", "warn", "when", "while", "xor",
];

pub const KW_R: &[&str] = &[
    "NA", "NULL", "TRUE", "FALSE", "Inf", "NaN", "aggregate", "apply", "array", "as.character",
    "as.data.frame", "as.factor", "as.numeric", "attach", "break", "c", "cat", "cbind",
    "colnames", "complete.cases", "cor", "cov", "data.frame", "det", "dim", "do.call",
    "duplicate", "else", "factor", "FALSE", "for", "function", "gc", "getwd", "head", "if",
    "in", "is.na", "is.null", "is.numeric", "lapply", "length", "library", "list", "lm", "ls",
    "matrix", "mean", "median", "merge", "min", "max", "na.omit", "names", "ncol", "next",
    "nrow", "order", "paste", "paste0", "plot", "print", "quantile", "rbind", "read.csv",
    "read.table", "repeat", "rep", "require", "reshape", "return", "rm", "runif", "rnorm",
    "sample", "sapply", "sd", "seq", "setwd", "sort", "source", "sum", "summary", "tapply",
    "TRUE", "unique", "var", "while", "with", "write.csv",
];

pub const KW_JULIA: &[&str] = &[
    "AbstractString", "Any", "Array", "Bool", "Channel", "Dict", "Float64", "Function", "Int",
    "Integer", "Matrix", "Number", "Set", "String", "Symbol", "Task", "Tuple", "Vector",
    "abstract", "async", "await", "baremodule", "begin", "break", "catch", "const", "continue",
    "deepcopy", "do", "eltype", "end", "enumerate", "enumerate", "export", "false", "filter",
    "finally", "for", "function", "global", "if", "import", "in", "isnothing", "let", "length",
    "local", "map", "missing", "module", "mutable", "nothing", "primitive", "quote", "return",
    "struct", "true", "try", "undef", "using", "var", "where", "while", "yield",
];

pub const KW_LUA: &[&str] = &[
    "and", "assert", "break", "collectgarbage", "coroutine", "do", "else", "elseif", "end",
    "error", "false", "for", "function", "goto", "if", "in", "ipairs", "local", "nil", "not",
    "or", "pairs", "pcall", "print", "repeat", "require", "return", "self", "table", "then",
    "true", "type", "until", "while", "xpcall",
];

pub const KW_HASKELL: &[&str] = &[
    "Applicative", "Bool", "Either", "Eq", "False", "Foldable", "Functor", "IO", "Int", "Just",
    "Left", "Maybe", "Monad", "Monoid", "Nothing", "Ord", "Right", "Semigroup", "Show", "String",
    "True", "all", "and", "any", "case", "class", "concat", "data", "default", "deriving", "do",
    "drop", "else", "elem", "error", "filter", "flip", "foldl", "foldr", "for", "forall",
    "foreign", "fst", "head", "if", "import", "in", "infix", "infixl", "infixr", "instance",
    "last", "length", "let", "lines", "lookup", "main", "map", "mdo", "mod", "module", "newtype",
    "not", "null", "of", "otherwise", "print", "proc", "putStr", "putStrLn", "rec", "reverse",
    "seq", "show", "snd", "splitAt", "take", "then", "type", "undefined", "where", "zip",
];

pub const KW_LISP: &[&str] = &[
    "apply", "assoc", "atom", "begin", "car", "case", "catch", "cdr", "char", "class", "cond",
    "cons", "count", "declare", "def", "default", "defclass", "defconstant", "defmacro",
    "defmethod", "defmulti", "defn", "defonce", "defpackage", "defparameter", "defprotocol",
    "defrecord", "defstruct", "deftest", "deftype", "defun", "defvar", "do", "doseq", "dotimes",
    "doto", "doto", "eval", "every", "false", "finally", "first", "fn", "for", "format", "if",
    "in-ns", "inc", "intern", "is", "lambda", "lazy", "let", "letfn", "list", "load", "loop",
    "macroexpand", "map", "mapcat", "merge", "name", "new", "next", "not", "ns", "nth", "nil",
    "or", "partial", "print", "printf", "println", "quote", "recur", "reduce", "require",
    "rest", "second", "select", "seq", "some", "str", "string", "swap", "test", "throw", "true",
    "try", "unquote", "use", "var", "when", "while", "with-open", "yield", "zipmap",
];

pub const KW_OCAML: &[&str] = &[
    "Array", "Bool", "Buffer", "Bytes", "Char", "Complex", "Either", "Error", "Float", "Hashtbl",
    "Int32", "Int64", "Lazy", "List", "Map", "Nativeint", "None", "Ok", "Option", "Printexc",
    "Printf", "Queue", "Random", "Result", "Scanf", "Seq", "Set", "Some", "Stack", "String",
    "Sys", "Thread", "Unix", "and", "as", "assert", "begin", "class", "constraint", "do",
    "done", "downto", "else", "end", "exception", "external", "false", "for", "fun", "function",
    "functor", "if", "in", "include", "inherit", "initializer", "lazy", "let", "match", "method",
    "mod", "module", "mutable", "new", "nonrec", "object", "of", "open", "or", "private", "rec",
    "sig", "struct", "then", "to", "true", "try", "type", "val", "virtual", "when", "while",
    "with",
];

pub const KW_FSHARP: &[&str] = &[
    "abstract", "and", "as", "assert", "async", "await", "base", "begin", "by", "catch",
    "class", "default", "delegate", "do", "done", "downcast", "downto", "elif", "else", "end",
    "enum", "event", "exception", "extern", "false", "finally", "for", "fun", "function", "get",
    "global", "if", "in", "include", "inherit", "inline", "interface", "internal", "lazy",
    "let", "match", "member", "module", "mutable", "namespace", "new", "not", "null", "of",
    "open", "or", "override", "param", "private", "process", "protected", "public", "rec",
    "return", "select", "set", "sig", "static", "struct", "task", "then", "to", "true", "try",
    "type", "upcast", "use", "using", "val", "void", "when", "while", "with", "yield",
];

pub const KW_ERLANG: &[&str] = &[
    "after", "and", "andalso", "band", "begin", "bnot", "bor", "bsl", "bsr", "bxor", "case",
    "catch", "cond", "define", "div", "else", "end", "endif", "error", "export", "fun", "if",
    "import", "include", "include_lib", "is_atom", "is_binary", "is_boolean", "is_float",
    "is_function", "is_integer", "is_list", "is_map", "is_number", "is_pid", "is_port",
    "is_record", "is_reference", "is_tuple", "module", "nil", "not", "of", "ok", "or", "orelse",
    "receive", "record", "rem", "try", "true", "false", "undef", "undefined", "when", "xor",
];

pub const KW_ELIXIR: &[&str] = &[
    "Agent", "Atom", "Enum", "Float", "GenServer", "Integer", "Keyword", "List", "Map", "Node",
    "Process", "Range", "Registry", "Set", "Stream", "String", "Supervisor", "Task", "Tuple",
    "after", "alias", "and", "async", "await", "case", "catch", "cond", "def", "defdelegate",
    "defexception", "defguard", "defimpl", "defmacro", "defmodule", "defprotocol", "defp",
    "defstruct", "do", "else", "end", "false", "fn", "for", "if", "import", "in", "inspect",
    "nil", "not", "or", "quote", "raise", "receive", "require", "rescue", "reraise", "self",
    "send", "spawn", "spawn_link", "throw", "true", "try", "unless", "unquote", "use", "when",
    "with",
];

pub const KW_DART: &[&str] = &[
    "abstract", "as", "assert", "async", "await", "base", "break", "case", "catch", "class",
    "const", "continue", "covariant", "default", "deferred", "do", "dynamic", "else", "enum",
    "export", "extends", "extension", "external", "factory", "false", "final", "finally", "for",
    "get", "if", "implements", "import", "in", "interface", "is", "late", "library", "mixin",
    "new", "null", "of", "on", "operator", "part", "required", "rethrow", "return", "sealed",
    "set", "show", "static", "super", "switch", "sync", "this", "throw", "true", "try",
    "typedef", "var", "void", "when", "while", "with", "yield",
];

pub const KW_SOLIDITY: &[&str] = &[
    "abstract", "address", "as", "assembly", "assert", "bool", "break", "bytes", "calldata",
    "catch", "constant", "constructor", "continue", "contract", "delete", "do", "else", "emit",
    "enum", "event", "external", "fallback", "false", "for", "function", "if", "immutable",
    "import", "in", "indexed", "interface", "internal", "is", "let", "library", "mapping",
    "memory", "modifier", "new", "override", "package", "payable", "pragma", "private", "public",
    "pure", "receive", "require", "return", "returns", "revert", "solidity", "storage",
    "string", "struct", "super", "switch", "this", "throw", "true", "try", "type", "unchecked",
    "using", "var", "view", "virtual", "while",
];

pub const KW_ZIG: &[&str] = &[
    "addrspace", "align", "allowzero", "and", "anyframe", "anytype", "anyerror", "asm", "async",
    "await", "bool", "break", "callconv", "catch", "comptime", "const", "continue", "defer",
    "else", "enum", "errdefer", "error", "export", "extern", "false", "f16", "f32", "f64",
    "f80", "f128", "for", "if", "inline", "i8", "i16", "i32", "i64", "i128", "isize", "linksection",
    "noalias", "noinline", "nosuspend", "null", "opaque", "or", "orelse", "packed", "pub",
    "resume", "return", "self", "suspend", "switch", "test", "threadlocal", "true", "try",
    "type", "u8", "u16", "u32", "u64", "u128", "undefined", "union", "unreachable", "unsafe",
    "use", "usingnamespace", "var", "void", "volatile", "where", "while",
];

pub const KW_NIM: &[&str] = &[
    "addr", "and", "as", "asm", "bind", "block", "break", "case", "cast", "concept", "const",
    "continue", "converter", "defer", "discard", "distinct", "div", "do", "echo", "elif", "else",
    "end", "enum", "except", "export", "finally", "for", "from", "func", "if", "import", "in",
    "include", "interface", "is", "iterator", "let", "macro", "method", "mixin", "mod", "nil",
    "not", "notin", "object", "of", "or", "out", "proc", "ptr", "raise", "ref", "result",
    "return", "shl", "shr", "static", "template", "true", "false", "try", "tuple", "type",
    "using", "var", "when", "while", "with", "without", "xor", "yield",
];

pub const KW_D: &[&str] = &[
    "abstract", "alias", "align", "asm", "assert", "auto", "body", "bool", "break", "byte",
    "case", "cast", "catch", "class", "const", "continue", "debug", "default", "delegate",
    "delete", "deprecated", "do", "double", "else", "enum", "export", "extern", "false", "final",
    "finally", "float", "for", "foreach", "foreach_reverse", "function", "goto", "if",
    "immutable", "import", "in", "inout", "int", "interface", "is", "lazy", "long", "macro",
    "mixin", "module", "new", "nothrow", "null", "out", "override", "package", "pragma",
    "private", "protected", "public", "pure", "ref", "return", "scope", "shared", "short",
    "size_t", "static", "struct", "super", "switch", "synchronized", "template", "this", "throw",
    "true", "try", "type", "typeof", "ubyte", "uint", "ulong", "union", "unittest", "ushort",
    "version", "void", "volatile", "wchar", "while", "with",
];

pub const KW_V: &[&str] = &[
    "as", "asm", "assert", "atomic", "bool", "break", "const", "continue", "defer", "else",
    "enum", "false", "fn", "for", "go", "goto", "if", "import", "in", "interface", "is", "lock",
    "match", "module", "mut", "nil", "none", "or", "pub", "return", "rlock", "select", "shared",
    "spawn", "static", "struct", "super", "true", "type", "typeof", "union", "unsafe", "voidptr",
];

pub const KW_CRYSTAL: &[&str] = &[
    "abstract", "alias", "and", "annotation", "as", "asm", "begin", "break", "case", "class",
    "def", "do", "else", "elsif", "end", "ensure", "enum", "extend", "false", "for", "fun",
    "getter", "if", "import", "include", "in", "instance_sizeof", "is_a", "lib", "macro",
    "module", "next", "nil", "not", "of", "or", "out", "pointerof", "private", "property",
    "protected", "require", "rescue", "return", "select", "self", "setter", "sizeof", "spawn",
    "struct", "super", "then", "this", "throw", "true", "type", "typeof", "uninitialized",
    "union", "unless", "until", "verbatim", "when", "while", "with", "yield",
];

pub const KW_GDSCRIPT: &[&str] = &[
    "and", "as", "assert", "await", "break", "case", "class", "class_name", "const", "continue",
    "elif", "else", "enum", "extends", "false", "for", "func", "if", "in", "is", "match",
    "null", "onready", "or", "pass", "preload", "return", "self", "setget", "signal", "static",
    "super", "true", "var", "while", "yield",
];

pub const KW_FORTRAN: &[&str] = &[
    "abstract", "allocatable", "allocate", "assign", "associate", "backspace", "bind", "block",
    "call", "case", "character", "class", "close", "complex", "concurrent", "contains",
    "continue", "cycle", "data", "deallocate", "default", "deferred", "dimension", "do",
    "double", "else", "elseif", "end", "enddo", "endfile", "endif", "endinterface", "endmodule",
    "endprogram", "endselect", "endsubroutine", "endtype", "entry", "enumerator", "equivalence",
    "exit", "extends", "external", "false", "final", "flush", "forall", "format", "function",
    "generic", "go", "goto", "if", "implicit", "import", "in", "include", "inout", "inquire",
    "integer", "intent", "interface", "intrinsic", "is", "kind", "len", "logical", "module",
    "none", "non_overridable", "nopass", "null", "nullify", "only", "open", "operator", "optional",
    "out", "parameter", "pass", "pause", "pointer", "precision", "print", "private", "procedure",
    "program", "protected", "public", "pure", "read", "real", "recursive", "result", "return",
    "rewind", "save", "select", "sequence", "stop", "subroutine", "target", "then", "to", "true",
    "type", "use", "value", "volatile", "wait", "where", "while", "write",
];

pub const KW_MATLAB: &[&str] = &[
    "break", "case", "catch", "cell", "char", "classdef", "clear", "close", "continue", "disp",
    "double", "else", "elseif", "end", "enumeration", "error", "events", "false", "figure",
    "for", "format", "fprintf", "function", "get", "global", "if", "input", "int16", "int32",
    "int64", "int8", "length", "logical", "methods", "otherwise", "parfor", "persistent",
    "plot", "printf", "properties", "read", "return", "set", "size", "spmd", "string", "switch",
    "true", "try", "uint16", "uint32", "uint64", "uint8", "while", "write",
];

pub const KW_TEX: &[&str] = &[
    "addcontentsline", "appendix", "author", "begin", "bibitem", "bibliography",
    "bibliographystyle", "caption", "chapter", "cite", "documentclass", "end", "emph",
    "footnote", "frac", "include", "includegraphics", "input", "item", "label", "maketitle",
    "newcommand", "newenvironment", "newpage", "newline", "pageref", "paragraph", "part",
    "providecommand", "ref", "renewcommand", "rule", "section", "setcounter", "subparagraph",
    "subsection", "subsubsection", "textbf", "textit", "texttt", "title", "underline", "url",
    "usepackage", "vspace", "hspace",
];

pub const KW_VB: &[&str] = &[
    "AddHandler", "AddressOf", "Alias", "And", "AndAlso", "As", "Boolean", "ByRef", "ByVal",
    "Call", "Case", "Catch", "CBool", "CByte", "CChar", "CDate", "CDbl", "CDec", "Char", "CInt",
    "Class", "CLng", "CObj", "Const", "Continue", "CSByte", "CShort", "CSng", "CStr", "CType",
    "CUInt", "CULng", "CUShort", "Date", "Decimal", "Declare", "Default", "Delegate", "Dim",
    "DirectCast", "Do", "Double", "Each", "Else", "ElseIf", "End", "EndIf", "Enum", "Erase",
    "Error", "Event", "Exit", "False", "Finally", "For", "Friend", "Function", "Get", "GetType",
    "Global", "GoSub", "GoTo", "Handles", "If", "Implements", "Imports", "In", "Inherits",
    "Integer", "Interface", "Is", "IsNot", "Let", "Lib", "Like", "Long", "Loop", "Me", "Mod",
    "Module", "MustInherit", "MustOverride", "MyBase", "MyClass", "Namespace", "New", "Next",
    "Not", "Nothing", "NotInheritable", "NotOverridable", "Object", "Of", "On", "Operator",
    "Option", "Optional", "Or", "OrElse", "Overloads", "Overridable", "Overrides", "ParamArray",
    "Partial", "Private", "Property", "Protected", "Public", "RaiseEvent", "ReadOnly", "ReDim",
    "REM", "RemoveHandler", "Resume", "Return", "SByte", "Select", "Set", "Shadows", "Shared",
    "Short", "Single", "Static", "Step", "Stop", "String", "Structure", "Sub", "SyncLock",
    "Then", "Throw", "To", "True", "Try", "TryCast", "TypeOf", "UInteger", "ULong", "UShort",
    "Using", "Variant", "Wend", "When", "While", "With", "WithEvents", "WriteOnly", "Xor",
];

pub const KW_POWERSHELL: &[&str] = &[
    "begin", "break", "catch", "class", "continue", "data", "do", "dynamicparam", "else",
    "elseif", "end", "enum", "exit", "filter", "finally", "for", "foreach", "from", "function",
    "if", "in", "param", "process", "return", "switch", "throw", "trap", "try", "until",
    "using", "var", "while", "workflow",
];

pub const KW_ASM: &[&str] = &[
    "aaa", "aad", "aam", "aas", "adc", "add", "addr", "align", "and", "assume", "bsf", "bsr",
    "bswap", "bt", "btc", "btr", "bts", "call", "cbw", "cdq", "clc", "cld", "cli", "cmc",
    "cmp", "cmps", "cmpxchg", "cwd", "cwde", "daa", "das", "db", "dd", "dec", "div", "dq", "dt",
    "dw", "endm", "endp", "ends", "equ", "extern", "f2xm1", "fabs", "fadd", "fbld", "fbstp",
    "fchs", "fclex", "fcom", "fcomp", "fcompp", "fdisi", "fdiv", "fdivr", "fdivrp", "feni",
    "ffree", "fiadd", "ficom", "ficomp", "fidiv", "fidivr", "fild", "fimul", "fincstp", "finit",
    "fist", "fistp", "fisub", "fisubr", "fld", "fldcw", "fldenv", "fmul", "fnop", "fnsave",
    "fnstcw", "fnstenv", "fnstsw", "fpatan", "fprem", "fptan", "frndint", "frstor", "fsave",
    "fscale", "fsqrt", "fst", "fstp", "fsub", "fsubr", "fsubrp", "ftst", "fwait", "fxam",
    "fxch", "fxtract", "fyl2x", "fyl2xp1", "global", "hlt", "idiv", "imul", "in", "inc",
    "incbin", "ins", "int", "into", "invd", "invlpg", "iret", "ja", "jae", "jb", "jbe", "jcxz",
    "je", "jecxz", "jg", "jge", "jl", "jle", "jmp", "jna", "jnae", "jnb", "jnbe", "jnc", "jne",
    "jng", "jnge", "jnl", "jnle", "jno", "jnp", "jns", "jnz", "jo", "jp", "jpe", "jpo", "jrcxz",
    "js", "jz", "lahf", "lds", "lea", "leave", "les", "lfs", "lgs", "lods", "loop", "loope",
    "loopne", "loopnz", "loopz", "lss", "macro", "mov", "movs", "movsx", "movsxd", "movzx",
    "mul", "near", "neg", "nop", "not", "offset", "or", "org", "out", "outs", "pop", "popa",
    "popad", "popf", "popfd", "proc", "ptr", "push", "pusha", "pushad", "pushf", "pushfd",
    "rcl", "rcr", "rep", "repe", "repne", "repnz", "repz", "resb", "resd", "resq", "rest",
    "resw", "ret", "retn", "retf", "rol", "ror", "sahf", "sal", "sar", "sbb", "scas", "section",
    "segment", "seta", "setae", "setb", "setbe", "setc", "sete", "setg", "setge", "setl",
    "setle", "setna", "setnae", "setnb", "setnbe", "setnc", "setne", "setng", "setnge", "setnl",
    "setnle", "setno", "setnp", "setns", "setnz", "seto", "setp", "setpe", "setpo", "sets",
    "setz", "sgdt", "shl", "shr", "sidt", "sldt", "smsw", "stc", "std", "sti", "str", "stos",
    "sub", "test", "times", "wait", "xadd", "xchg", "xlat", "xor",
];

pub const KW_GRAPHQL: &[&str] = &[
    "directive", "enum", "extend", "extends", "fragment", "implements", "input", "interface",
    "mutation", "null", "on", "query", "repeatable", "scalar", "schema", "subscription",
    "true", "false", "type", "union",
];

pub const KW_PROTO: &[&str] = &[
    "bool", "bytes", "double", "enum", "extend", "extensions", "fixed32", "fixed64", "float",
    "import", "int32", "int64", "map", "message", "oneof", "option", "optional", "package",
    "repeated", "required", "reserved", "returns", "rpc", "service", "sfixed32", "sfixed64",
    "sint32", "sint64", "stream", "string", "syntax", "to", "uint32", "uint64",
];

pub const KW_NIX: &[&str] = &[
    "abort", "assert", "builtins", "derivation", "else", "false", "fetchTarball", "fetchgit",
    "fetchurl", "if", "import", "in", "inherit", "let", "lib", "mkDerivation", "null", "or",
    "rec", "stdenv", "then", "throw", "true", "with",
];

pub const KW_HCL: &[&str] = &[
    "backend", "connection", "content", "count", "data", "default", "depends_on", "description",
    "dynamic", "for_each", "lifecycle", "locals", "module", "nullable", "output", "provider",
    "provisioner", "required_providers", "required_version", "resource", "sensitive", "terraform",
    "type", "validation", "value", "variable",
];

pub const KW_PASCAL: &[&str] = &[
    "absolute", "and", "array", "as", "asm", "assembler", "begin", "case", "cdecl", "class",
    "const", "constructor", "contains", "continue", "default", "destructor", "dispinterface",
    "div", "do", "downto", "dynamic", "else", "end", "except", "exports", "external", "false",
    "far", "file", "finalization", "finally", "for", "forward", "function", "goto", "if",
    "implementation", "implements", "in", "inherited", "initialization", "inline", "interface",
    "is", "label", "library", "mod", "near", "nil", "not", "object", "of", "on", "operator",
    "or", "out", "packed", "pascal", "private", "procedure", "program", "property", "protected",
    "public", "published", "raise", "read", "record", "register", "repeat", "requires",
    "resourcestring", "safecall", "sealed", "self", "set", "shl", "shr", "stdcall", "stored",
    "string", "then", "threadvar", "to", "true", "try", "type", "unit", "unsafe", "uses", "var",
    "virtual", "while", "with", "write", "xor",
];

pub const KW_COBOL: &[&str] = &[
    "accept", "add", "address", "after", "all", "alter", "and", "are", "area", "ascending",
    "assign", "at", "author", "before", "binary", "blank", "block", "by", "call", "cancel",
    "cd", "cf", "ch", "clock-units", "close", "cobol", "code", "col", "collating", "com-reg",
    "comp", "comp-1", "comp-2", "comp-3", "comp-4", "comp-5", "computational", "compute",
    "configuration", "constant", "contains", "content", "continue", "control", "converting",
    "copy", "corr", "corresponding", "count", "currency", "data", "date", "date-compiled",
    "date-written", "day", "day-of-week", "dbcs", "de", "debug-contents", "debug-item",
    "debug-line", "debug-name", "debug-sub-1", "debug-sub-2", "debug-sub-3", "debugging",
    "decimal-point", "declaratives", "delete", "delimited", "delimiter", "depending",
    "descending", "destination", "detail", "disable", "display", "divide", "division", "down",
    "duplicates", "dynamic", "egcs", "eject", "else", "emi", "enable", "end", "end-add",
    "end-call", "end-compute", "end-delete", "end-divide", "end-evaluate", "end-if",
    "end-multiply", "end-of-page", "end-perform", "end-read", "end-receive", "end-return",
    "end-rewrite", "end-search", "end-start", "end-string", "end-subtract", "end-unstring",
    "end-write", "ending", "enter", "entry", "environment", "eop", "equal", "equals", "error",
    "escape", "esi", "evaluate", "every", "examine", "exception", "exit", "explicit", "export",
    "extend", "external", "false", "fd", "file", "file-control", "file-id", "filler", "final",
    "first", "footing", "for", "foreground-color", "format", "free", "from", "function",
    "generate", "giving", "global", "go", "goback", "greater", "grid", "group", "heading",
    "high-value", "high-values", "i-o", "i-o-control", "id", "identification", "if", "in",
    "index", "indexed", "indicate", "initial", "initialize", "initiate", "input",
    "input-output", "inspect", "installation", "into", "intrinsic", "invalid", "is", "just",
    "justified", "kanji", "kept", "key", "keyboard", "label", "last", "leading", "left",
    "leftline", "length", "less", "limit", "limits", "linage", "linage-counter", "line",
    "line-counter", "lines", "linkage", "lock", "low-value", "low-values", "memory", "merge",
    "message", "metaclass", "mode", "modules", "more-labels", "move", "multiple", "multiply",
    "national", "native", "negative", "next", "no", "not", "null", "nulls", "number", "numeric",
    "numeric-edited", "object-computer", "occurs", "of", "off", "omitted", "on", "open",
    "optional", "or", "order", "organization", "other", "output", "overflow", "override",
    "packed-decimal", "padding", "page", "paragraph", "perform", "pf", "ph", "pic", "picture",
    "plus", "pointer", "position", "positive", "printing", "program", "program-id", "purge",
    "queue", "quote", "quotes", "random", "rd", "read", "receive", "record", "records",
    "recursive", "redefines", "reelfile-status", "reference", "relative", "release", "remainder",
    "remarks", "removal", "renames", "replace", "replacing", "report", "reporting", "reports",
    "required", "rerun", "reserve", "reset", "return", "returning", "reverse", "reverse-video",
    "reversed", "rewind", "rewrite", "rf", "rh", "right", "rounded", "run", "same", "screen",
    "sd", "search", "section", "secure", "security", "seek", "segment", "segment-limit",
    "select", "self", "send", "sentence", "separate", "sequence", "sequential", "set", "shading",
    "shadow", "sign", "size", "skip1", "skip2", "skip3", "sort", "sort-merge", "source",
    "source-computer", "space", "spaces", "special-names", "standard", "standard-1",
    "standard-2", "start", "status", "stop", "string", "sub-queue-1", "sub-queue-2",
    "sub-queue-3", "subtract", "sum", "suppress", "symbolic", "sync", "synchronized", "table",
    "tallying", "tape", "tennis", "terminal", "test", "than", "then", "through", "thru", "time",
    "times", "to", "top", "trace", "trailing", "true", "type", "unit", "unstring", "until", "up",
    "upon", "usage", "use", "using", "value", "values", "varying", "when", "while", "with",
    "words", "working-storage", "write", "write-only", "zero", "zeroes", "zeros",
];

pub const KW_TYPST: &[&str] = &[
    "auto", "align", "block", "box", "break", "circle", "code", "columns", "context", "continue",
    "else", "enum", "eval", "figure", "for", "footnote", "grid", "heading", "highlight", "if",
    "image", "import", "in", "include", "let", "line", "link", "list", "locate", "measure",
    "none", "not", "page", "pad", "par", "path", "place", "query", "quote", "raw", "read",
    "rect", "return", "rotate", "scale", "set", "show", "stack", "strike", "strong", "table",
    "text", "true", "false", "underline", "while", "with",
];
// ---------------------------------------------------------------------------
// Spec table
// ---------------------------------------------------------------------------

const OCAML_SPEC: LangSpec = spec(CommentStyle::ParenStar, KW_OCAML);
const FSHARP_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_FSHARP);
const SQL_SPEC: LangSpec = LangSpec {
    comments: CommentStyle::Dash,
    keywords: KW_SQL,
    ignore_case: true,
};

const YAML_SPEC: LangSpec = spec(CommentStyle::Hash, &[]);
const INI_SPEC: LangSpec = spec(CommentStyle::Semicolon, &[]);
const CSS_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_CSS);
const C_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_C);
const JVM_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_JVM);
const VB_SPEC: LangSpec = spec(CommentStyle::Apostrophe, KW_VB);
const JS_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_JS);
const DART_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_DART);
const RUST_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_RUST);
const GO_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_GO);
const SWIFT_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_SWIFT);
const ZIG_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_ZIG);
const NIM_SPEC: LangSpec = spec(CommentStyle::Hash, KW_NIM);
const D_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_D);
const V_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_V);
const SOLIDITY_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_SOLIDITY);
const CRYSTAL_SPEC: LangSpec = spec(CommentStyle::Hash, KW_CRYSTAL);
const GDSCRIPT_SPEC: LangSpec = spec(CommentStyle::Hash, KW_GDSCRIPT);
const PASCAL_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_PASCAL);
const COBOL_SPEC: LangSpec = spec(CommentStyle::None, KW_COBOL);
const FORTRAN_SPEC: LangSpec = spec(CommentStyle::Bang, KW_FORTRAN);
const PYTHON_SPEC: LangSpec = spec(CommentStyle::Hash, KW_PYTHON);
const RUBY_SPEC: LangSpec = spec(CommentStyle::Hash, KW_RUBY);
const PERL_SPEC: LangSpec = spec(CommentStyle::Hash, KW_PERL);
const R_SPEC: LangSpec = spec(CommentStyle::Hash, KW_R);
const JULIA_SPEC: LangSpec = spec(CommentStyle::Hash, KW_JULIA);
const LUA_SPEC: LangSpec = spec(CommentStyle::Dash, KW_LUA);
const PHP_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_PHP);
const PS_SPEC: LangSpec = spec(CommentStyle::Hash, KW_POWERSHELL);
const ELIXIR_SPEC: LangSpec = spec(CommentStyle::Hash, KW_ELIXIR);
const ERLANG_SPEC: LangSpec = spec(CommentStyle::Percent, KW_ERLANG);
const SHELL_SPEC: LangSpec = spec(CommentStyle::Hash, KW_SHELL);
const MAKE_SPEC: LangSpec = spec(CommentStyle::Hash, &[]);
const HASKELL_SPEC: LangSpec = spec(CommentStyle::Dash, KW_HASKELL);
const LISP_SPEC: LangSpec = spec(CommentStyle::Semicolon, KW_LISP);
const GRAPHQL_SPEC: LangSpec = spec(CommentStyle::Hash, KW_GRAPHQL);
const PROTO_SPEC: LangSpec = spec(CommentStyle::CStyle, KW_PROTO);
const MATLAB_SPEC: LangSpec = spec(CommentStyle::Percent, KW_MATLAB);
const TEX_SPEC: LangSpec = spec(CommentStyle::Percent, KW_TEX);
const TYPST_SPEC: LangSpec = spec(CommentStyle::None, KW_TYPST);
const ASM_SPEC: LangSpec = spec(CommentStyle::Semicolon, KW_ASM);
const HCL_SPEC: LangSpec = spec(CommentStyle::Hash, KW_HCL);
const NIX_SPEC: LangSpec = spec(CommentStyle::Hash, KW_NIX);
const CSTYLE_EMPTY_SPEC: LangSpec = spec(CommentStyle::CStyle, &[]);
const DASH_EMPTY_SPEC: LangSpec = spec(CommentStyle::Dash, &[]);

/// Normalize a fence language tag: lowercase, drop leading dots/dashes, and
/// fold `c++`/`c#`/`f#`-style spellings to their short aliases.
fn normalize(lang: &str) -> String {
    lang.trim()
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .replace("++", "pp")
        .replace('#', "sharp")
        .replace(['-', '_'], "")
}

/// Resolve a fence language tag to its highlighting spec. Unknown tags fall
/// back to C-style comments with no keywords.
pub fn lang_spec(lang: &str) -> &'static LangSpec {
    let l = normalize(lang);
    match l.as_str() {
        // Data formats & markup: strings/numbers/literals only.
        "json" | "json5" | "geojson" | "jsonl" | "ndjson" => &PLAIN,
        "jsonc" => &JSONC,
        "csv" | "tsv" | "diff" | "patch" => &PLAIN,
        "html" | "htm" | "xhtml" | "xml" | "svg" | "rss" | "atom" | "plist" | "vue" | "svelte"
        | "astro" | "ejs" | "erb" | "haml" | "pug" | "jade" | "mustache" | "handlebars" | "hbs"
        | "jsp" | "aspx" | "cshtml" => &PLAIN,
        "yaml" | "yml" | "raml" => &YAML_SPEC,
        "toml" | "ini" | "cfg" | "conf" | "env" | "dotenv" | "properties" | "editorconfig" => {
            &INI_SPEC
        }

        // Stylesheet languages.
        "css" | "scss" | "sass" | "less" | "stylus" | "styl" | "postcss" | "pcss" => &CSS_SPEC,

        // C family.
        "c" | "h" | "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" | "objc" | "objectivec"
        | "objcpp" | "mm" => &C_SPEC,

        // JVM & .NET.
        "java" | "kt" | "kts" | "kotlin" | "scala" | "sc" | "groovy" | "gvy" | "gradle" | "cs"
        | "csharp" => &JVM_SPEC,
        "fs" | "fsi" | "fsx" | "fsharp" => &FSHARP_SPEC,
        "vb" | "vba" | "vbnet" | "vbscript" | "vbs" => &VB_SPEC,

        // JS / TS / Dart.
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "javascript" | "typescript" => &JS_SPEC,
        "dart" => &DART_SPEC,

        // Systems languages.
        "rs" | "rust" => &RUST_SPEC,
        "go" | "golang" => &GO_SPEC,
        "swift" => &SWIFT_SPEC,
        "zig" => &ZIG_SPEC,
        "nim" | "nimble" | "nims" => &NIM_SPEC,
        "d" | "di" => &D_SPEC,
        "v" | "vlang" => &V_SPEC,
        "sol" | "solidity" => &SOLIDITY_SPEC,
        "cr" | "crystal" => &CRYSTAL_SPEC,
        "gd" | "gdscript" => &GDSCRIPT_SPEC,
        "pas" | "pp" | "lpr" | "dpr" | "pascal" | "delphi" | "freepascal" | "objectpascal" => {
            &PASCAL_SPEC
        }
        "cobol" | "cbl" | "cob" | "cpy" => &COBOL_SPEC,
        "f" | "for" | "f77" | "f90" | "f95" | "f03" | "f08" | "f18" | "fortran" | "ftn" => {
            &FORTRAN_SPEC
        }

        // Scripting & dynamic languages.
        "py" | "pyw" | "pyi" | "python" | "bazel" | "starlark" | "bzl" => &PYTHON_SPEC,
        "rb" | "ruby" | "rake" | "gemspec" | "podspec" | "irb" => &RUBY_SPEC,
        "pl" | "pm" | "perl" => &PERL_SPEC,
        "r" => &R_SPEC,
        "jl" | "julia" => &JULIA_SPEC,
        "lua" => &LUA_SPEC,
        "php" | "php3" | "php4" | "php5" | "phtml" => &PHP_SPEC,
        "ps1" | "psm1" | "psd1" | "powershell" => &PS_SPEC,
        "bat" | "cmd" | "batch" | "dosbatch" => &PLAIN,
        "ex" | "exs" | "elixir" | "eex" | "heex" | "leex" => &ELIXIR_SPEC,
        "erl" | "hrl" | "erlang" => &ERLANG_SPEC,

        // Shell languages.
        "sh" | "bash" | "zsh" | "fish" | "shell" | "ksh" | "csh" | "tcsh" | "dash" | "ash"
        | "mksh" => &SHELL_SPEC,
        "makefile" | "make" | "mk" | "cmake" | "dockerfile" | "containerfile" | "ninja" => {
            &MAKE_SPEC
        }

        // ML family & friends.
        "hs" | "lhs" | "haskell" | "elm" | "purs" | "purescript" | "idr" | "idris" | "agda"
        | "lagda" => &HASKELL_SPEC,
        "ml" | "mli" | "ocaml" | "eliom" => &OCAML_SPEC,
        "cl" | "lisp" | "elisp" | "emacs" | "emacs-lisp" | "scheme" | "scm" | "ss" | "racket"
        | "rkt" | "clojure" | "clj" | "cljs" | "cljc" | "edn" | "hy" | "fennel" | "janet" => {
            &LISP_SPEC
        }

        // Query & data-definition languages.
        "sql" | "mysql" | "mariadb" | "postgres" | "postgresql" | "pgsql" | "plsql" | "tsql"
        | "mssql" | "sqlite" | "sqlite3" | "hql" | "hiveql" | "bigquery" => &SQL_SPEC,
        "graphql" | "gql" => &GRAPHQL_SPEC,
        "proto" | "protobuf" | "thrift" | "avdl" => &PROTO_SPEC,

        // Scientific & document languages.
        "m" | "matlab" | "octave" => &MATLAB_SPEC,
        "tex" | "latex" | "sty" | "cls" | "bib" => &TEX_SPEC,
        "typ" | "typst" => &TYPST_SPEC,

        // Low-level & hardware.
        "asm" | "nasm" | "gas" | "att" | "intel" | "s" | "a51" | "inc" => &ASM_SPEC,
        "vhdl" | "vhd" => &DASH_EMPTY_SPEC,
        "verilog" | "sv" | "systemverilog" => &CSTYLE_EMPTY_SPEC,

        // Config & infrastructure.
        "tf" | "hcl" | "terraform" | "nomad" => &HCL_SPEC,
        "nix" => &NIX_SPEC,
        "jsonnet" | "libsonnet" | "cue" => &CSTYLE_EMPTY_SPEC,
        "rego" => &YAML_SPEC,

        _ => &GENERIC,
    }
}
