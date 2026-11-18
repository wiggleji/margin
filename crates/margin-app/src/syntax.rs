//! Light syntax highlighting for diffs and note code blocks.
//!
//! A small hand-written lexer per language family (no grammar files, so
//! the app stays small): comments, strings, numbers, keywords, types,
//! function calls and attributes. It works a line at a time and carries
//! block comments and multi-line strings to the next line through
//! [`State`]. Good enough to read code at a glance; it doesn't parse.

use crate::theme;
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    String,
    Number,
    Comment,
    Type,
    Function,
    Attr,
}

impl Kind {
    pub fn color(self) -> Color32 {
        let s = &theme::cur().syntax;
        match self {
            Kind::Keyword => s.keyword,
            Kind::String => s.string,
            Kind::Number => s.number,
            Kind::Comment => s.comment,
            Kind::Type => s.ty,
            Kind::Function => s.function,
            Kind::Attr => s.attr,
        }
    }
}

/// A token: byte range in the line, and its kind.
pub type Span = (usize, usize, Kind);

/// Carried from one line to the next: inside a block comment or a
/// multi-line string, until `end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Normal,
    Open { end: &'static str, kind: Kind },
}

#[derive(Debug)]
pub struct Lang {
    pub name: &'static str,
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters for one-line strings.
    quotes: &'static str,
    /// Delimiters of strings that may span lines (opening = closing).
    long_strings: &'static [&'static str],
    keywords: &'static [&'static str],
    /// true / null / self …
    constants: &'static [&'static str],
    /// Built-in type names.
    types: &'static [&'static str],
    /// Capitalised names are types (Rust, Java, TS, Swift …).
    caps_types: bool,
    /// Keywords match in any case (SQL).
    any_case: bool,
    /// `@name` is an attribute / decorator.
    at_attrs: bool,
    /// `name!(…)` is a macro call (Rust).
    bang_macros: bool,
    /// `#[…]` / `#![…]` attributes (Rust).
    hash_attrs: bool,
    /// `$name` variables (shell, PHP).
    dollar_vars: bool,
    /// Markup: `<tag attr="…">` (HTML, XML, Vue, SVG).
    markup: bool,
    /// `key:` / `key =` at the start of a line is a key (YAML, TOML, INI).
    keys: bool,
}

const BASE: Lang = Lang {
    name: "",
    line_comment: &[],
    block_comment: None,
    quotes: "\"'",
    long_strings: &[],
    keywords: &[],
    constants: &[],
    types: &[],
    caps_types: false,
    any_case: false,
    at_attrs: false,
    bang_macros: false,
    hash_attrs: false,
    dollar_vars: false,
    markup: false,
    keys: false,
};

const C_COMMENTS: Lang = Lang { line_comment: &["//"], block_comment: Some(("/*", "*/")), ..BASE };

static RUST: Lang = Lang {
    name: "rust",
    quotes: "\"",
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
        "pub", "ref", "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while", "yield",
    ],
    constants: &["true", "false", "self", "Self", "None", "Some", "Ok", "Err"],
    types: &["bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "String", "Vec", "Option", "Result", "Box"],
    caps_types: true,
    bang_macros: true,
    hash_attrs: true,
    ..C_COMMENTS
};

static JS: Lang = Lang {
    name: "javascript",
    quotes: "\"'",
    long_strings: &["`"],
    keywords: &[
        "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "finally", "for", "from", "function",
        "get", "if", "import", "in", "instanceof", "let", "new", "of", "return", "set", "static", "switch", "throw", "try", "typeof", "var", "void", "while", "with", "yield",
        // TypeScript
        "abstract", "as", "declare", "enum", "implements", "interface", "keyof", "namespace", "private", "protected", "public", "readonly", "satisfies", "type",
    ],
    constants: &["true", "false", "null", "undefined", "this", "super", "NaN", "Infinity"],
    types: &["string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint"],
    caps_types: true,
    at_attrs: true,
    ..C_COMMENTS
};

static GO: Lang = Lang {
    name: "go",
    quotes: "\"'",
    long_strings: &["`"],
    keywords: &[
        "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return",
        "select", "struct", "switch", "type", "var",
    ],
    constants: &["true", "false", "nil", "iota"],
    types: &["bool", "byte", "error", "float32", "float64", "int", "int8", "int16", "int32", "int64", "rune", "string", "uint", "uint8", "uint16", "uint32", "uint64", "uintptr", "any"],
    ..C_COMMENTS
};

static C_LIKE: Lang = Lang {
    name: "c",
    quotes: "\"'",
    keywords: &[
        // C / C++ / C# / Java / Kotlin / Swift / Dart / Scala: one shared list reads fine
        "abstract", "auto", "break", "case", "catch", "class", "const", "constexpr", "continue", "default", "defer", "delete", "do", "else", "enum", "explicit", "export", "extends",
        "extern", "final", "finally", "for", "fun", "func", "goto", "guard", "if", "implements", "import", "in", "inline", "interface", "internal", "is", "let", "namespace", "new",
        "object", "operator", "override", "package", "private", "protected", "public", "return", "sealed", "sizeof", "static", "struct", "switch", "template", "throw", "throws", "try",
        "typedef", "typename", "union", "using", "val", "var", "virtual", "void", "volatile", "when", "where", "while", "async", "await", "yield", "protocol", "extension", "init",
        "deinit", "self", "data", "companion", "suspend", "lateinit", "open", "fileprivate", "mutating", "#include", "#define", "#if", "#ifdef", "#ifndef", "#endif", "#else", "#pragma",
    ],
    constants: &["true", "false", "null", "nullptr", "nil", "NULL", "this", "super", "self", "Self"],
    types: &[
        "bool", "boolean", "byte", "char", "double", "float", "int", "long", "short", "signed", "unsigned", "size_t", "string", "String", "Int", "Long", "Double", "Float", "Bool",
        "Boolean", "Unit", "Any", "Object", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int8_t", "int16_t", "int32_t", "int64_t",
    ],
    caps_types: true,
    at_attrs: true,
    long_strings: &["\"\"\""],
    ..C_COMMENTS
};

static PYTHON: Lang = Lang {
    name: "python",
    line_comment: &["#"],
    quotes: "\"'",
    long_strings: &["\"\"\"", "'''"],
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "match", "case", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
    ],
    constants: &["True", "False", "None", "self", "cls"],
    types: &["int", "float", "str", "bool", "bytes", "list", "dict", "set", "tuple", "object"],
    caps_types: true,
    at_attrs: true,
    ..BASE
};

static RUBY: Lang = Lang {
    name: "ruby",
    line_comment: &["#"],
    quotes: "\"'",
    keywords: &[
        "alias", "and", "begin", "break", "case", "class", "def", "defined?", "do", "else", "elsif", "end", "ensure", "for", "if", "in", "module", "next", "not", "or", "redo", "rescue",
        "retry", "return", "then", "undef", "unless", "until", "when", "while", "yield", "require", "require_relative", "attr_reader", "attr_accessor", "private", "protected", "public",
    ],
    constants: &["true", "false", "nil", "self", "super"],
    caps_types: true,
    at_attrs: true,
    ..BASE
};

static SHELL: Lang = Lang {
    name: "shell",
    line_comment: &["#"],
    quotes: "\"'",
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "local", "export", "readonly", "set", "unset", "shift",
        "exit", "echo", "cd", "source", "eval", "exec", "trap",
    ],
    constants: &["true", "false"],
    dollar_vars: true,
    ..BASE
};

static SQL: Lang = Lang {
    name: "sql",
    line_comment: &["--"],
    block_comment: Some(("/*", "*/")),
    quotes: "'\"",
    keywords: &[
        "select", "from", "where", "and", "or", "not", "insert", "into", "values", "update", "set", "delete", "create", "table", "index", "view", "drop", "alter", "add", "column",
        "primary", "key", "foreign", "references", "join", "left", "right", "inner", "outer", "on", "as", "group", "by", "order", "having", "limit", "offset", "distinct", "union",
        "all", "case", "when", "then", "else", "end", "is", "in", "like", "between", "exists", "default", "unique", "constraint", "begin", "commit", "rollback", "with", "returning",
    ],
    constants: &["null", "true", "false"],
    types: &["int", "integer", "bigint", "smallint", "text", "varchar", "char", "boolean", "date", "timestamp", "timestamptz", "numeric", "decimal", "serial", "uuid", "jsonb", "json"],
    any_case: true,
    ..BASE
};

static LUA: Lang = Lang {
    name: "lua",
    line_comment: &["--"],
    quotes: "\"'",
    keywords: &["and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local", "not", "or", "repeat", "return", "then", "until", "while"],
    constants: &["true", "false", "nil", "self"],
    ..BASE
};

static CSS: Lang = Lang {
    name: "css",
    quotes: "\"'",
    keywords: &["important", "media", "import", "keyframes", "supports", "font-face", "include", "mixin", "extend", "use"],
    at_attrs: true,
    ..C_COMMENTS
};

static JSON: Lang = Lang { name: "json", quotes: "\"", constants: &["true", "false", "null"], ..C_COMMENTS };

static YAML: Lang = Lang { name: "yaml", line_comment: &["#"], quotes: "\"'", constants: &["true", "false", "null", "yes", "no", "on", "off", "~"], keys: true, ..BASE };

static TOML: Lang = Lang { name: "toml", line_comment: &["#", ";"], quotes: "\"'", long_strings: &["\"\"\"", "'''"], constants: &["true", "false"], keys: true, ..BASE };

static HTML: Lang = Lang { name: "html", block_comment: Some(("<!--", "-->")), quotes: "\"'", markup: true, ..BASE };

static DOCKER: Lang = Lang {
    name: "dockerfile",
    line_comment: &["#"],
    quotes: "\"'",
    keywords: &["FROM", "RUN", "CMD", "LABEL", "EXPOSE", "ENV", "ADD", "COPY", "ENTRYPOINT", "VOLUME", "USER", "WORKDIR", "ARG", "ONBUILD", "STOPSIGNAL", "HEALTHCHECK", "SHELL", "AS"],
    dollar_vars: true,
    ..BASE
};

static MAKE: Lang = Lang { name: "make", line_comment: &["#"], quotes: "\"'", keywords: &["ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "define", "endef", "export"], dollar_vars: true, ..BASE };

/// The language for a file path (by extension, or a few file names).
pub fn for_path(path: &str) -> Option<&'static Lang> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "dockerfile" | "containerfile" => return Some(&DOCKER),
        "makefile" | "gnumakefile" => return Some(&MAKE),
        "gemfile" | "rakefile" | "podfile" | "fastfile" => return Some(&RUBY),
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => return Some(&SHELL),
        _ => {}
    }
    if lower.starts_with("dockerfile.") {
        return Some(&DOCKER);
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e)?;
    let lang: &Lang = match ext {
        "rs" => &RUST,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => &JS,
        "go" => &GO,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" | "cs" | "java" | "kt" | "kts" | "scala" | "swift" | "dart" | "groovy" | "gradle" | "php" => &C_LIKE,
        "py" | "pyi" => &PYTHON,
        "rb" | "rake" | "gemspec" => &RUBY,
        "sh" | "bash" | "zsh" | "fish" | "envrc" => &SHELL,
        "sql" => &SQL,
        "lua" => &LUA,
        "css" | "scss" | "sass" | "less" => &CSS,
        "json" | "jsonc" | "json5" => &JSON,
        "yml" | "yaml" => &YAML,
        "toml" | "ini" | "cfg" | "conf" | "properties" => &TOML,
        "html" | "htm" | "xml" | "svg" | "vue" | "svelte" | "plist" | "xhtml" | "erb" => &HTML,
        _ => return None,
    };
    Some(lang)
}

/// The language for a Markdown code fence's info string (```rust, ```ts …).
pub fn for_fence(info: &str) -> Option<&'static Lang> {
    let tag = info.split_whitespace().next()?.to_ascii_lowercase();
    let ext = match tag.as_str() {
        "rust" => "rs",
        "javascript" | "node" => "js",
        "typescript" => "ts",
        "python" | "py3" => "py",
        "ruby" => "rb",
        "golang" => "go",
        "shell" | "console" | "shellscript" => "sh",
        "c++" => "cpp",
        "csharp" | "c#" => "cs",
        "kotlin" => "kt",
        "objc" | "objective-c" => "m",
        "dockerfile" | "docker" => return Some(&DOCKER),
        "makefile" | "make" => return Some(&MAKE),
        "jsonc" | "json5" => "json",
        "postgres" | "postgresql" | "mysql" | "sqlite" => "sql",
        other => other,
    };
    for_path(&format!("x.{ext}"))
}

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Tokens of one line. `state` comes in from the line before and goes out
/// to the next.
pub fn line(lang: &Lang, text: &str, state: &mut State) -> Vec<Span> {
    let mut out: Vec<Span> = vec![];
    // very long lines (minified files): leave plain
    if text.len() > 2000 {
        return out;
    }
    let b = text.as_bytes();
    let n = text.len();
    let mut i = 0;
    let at = |i: usize, s: &str| text[i..].starts_with(s);
    // finish what the line before left open
    if let State::Open { end, kind } = *state {
        match text.find(end) {
            Some(e) => {
                out.push((0, e + end.len(), kind));
                i = e + end.len();
                *state = State::Normal;
            }
            None => {
                out.push((0, n, kind));
                return out;
            }
        }
    }
    // YAML / TOML / INI: a key at the start of the line, or a [section]
    if lang.keys {
        let t = text.trim_start();
        let lead = n - t.len();
        if lead >= i {
            if t.starts_with('[') {
                if let Some(e) = t.find(']') {
                    out.push((lead, lead + e + 1, Kind::Type));
                    i = lead + e + 1;
                }
            } else {
                let body = t.strip_prefix("- ").unwrap_or(t);
                let off = lead + (t.len() - body.len());
                if let Some(k) = body.find([':', '=']) {
                    let key = body[..k].trim_end();
                    if !key.is_empty() && key.chars().all(|c| ident_char(c) || "-./".contains(c)) {
                        out.push((off, off + key.len(), Kind::Attr));
                        i = off + key.len();
                    }
                }
            }
        }
    }
    let mut in_tag = false;
    while i < n {
        let c = text[i..].chars().next().unwrap_or(' ');
        let cl = c.len_utf8();
        // comments
        if let Some(lc) = lang.line_comment.iter().find(|lc| at(i, lc)) {
            // `#` in shell needs a word boundary (`$#`, `a#b` aren't comments)
            let boundary = *lc != "#" || i == 0 || b[i - 1].is_ascii_whitespace() || lang.name == "python" || lang.name == "ruby" || lang.name == "yaml" || lang.name == "toml";
            if boundary {
                out.push((i, n, Kind::Comment));
                return out;
            }
        }
        if let Some((open, close)) = lang.block_comment
            && at(i, open)
        {
            match text[i + open.len()..].find(close) {
                Some(e) => {
                    let end = i + open.len() + e + close.len();
                    out.push((i, end, Kind::Comment));
                    i = end;
                }
                None => {
                    out.push((i, n, Kind::Comment));
                    *state = State::Open { end: close, kind: Kind::Comment };
                    return out;
                }
            }
            continue;
        }
        // markup tags: <name attr="v"> </name>
        if lang.markup {
            if c == '<' && text[i + 1..].starts_with(|c: char| c.is_alphabetic() || c == '/' || c == '!' || c == '?') {
                let s = i + 1 + usize::from(matches!(b.get(i + 1), Some(b'/' | b'!' | b'?')));
                let e = text[s..].find(|c: char| !(ident_char(c) || "-:.".contains(c))).map_or(n, |e| s + e);
                out.push((s, e, Kind::Keyword));
                in_tag = true;
                i = e;
                continue;
            }
            if in_tag && ident_start(c) {
                let e = text[i..].find(|c: char| !(ident_char(c) || "-:@.".contains(c))).map_or(n, |e| i + e);
                out.push((i, e, Kind::Attr));
                i = e;
                continue;
            }
            if c == '>' {
                in_tag = false;
            }
            // text between tags stays plain
            if !in_tag {
                i += cl;
                continue;
            }
        }
        // strings that may span lines
        if let Some(d) = lang.long_strings.iter().find(|d| at(i, d)) {
            match text[i + d.len()..].find(*d) {
                Some(e) => {
                    let end = i + d.len() + e + d.len();
                    out.push((i, end, Kind::String));
                    i = end;
                }
                None => {
                    out.push((i, n, Kind::String));
                    *state = State::Open { end: d, kind: Kind::String };
                    return out;
                }
            }
            continue;
        }
        // Rust: lifetimes and char literals both start with '
        if lang.name == "rust" && c == '\'' {
            let rest = &text[i + 1..];
            let is_char = rest.starts_with('\\') || rest.chars().nth(1) == Some('\'');
            if !is_char {
                let e = rest.find(|c: char| !ident_char(c)).map_or(n, |e| i + 1 + e);
                out.push((i, e, Kind::Attr));
                i = e.max(i + 1);
                continue;
            }
            // 'x' or '\n': up to the closing quote
            let skip = if rest.starts_with('\\') { 2 } else { rest.chars().next().map_or(1, char::len_utf8) };
            let e = rest.get(skip..).and_then(|r| r.find('\'')).map_or(n, |e| i + 1 + skip + e + 1);
            out.push((i, e, Kind::String));
            i = e;
            continue;
        }
        if lang.quotes.contains(c) {
            let mut j = i + 1;
            let mut closed = false;
            while j < n {
                match b[j] {
                    b'\\' => j += 2,
                    x if x == c as u8 => {
                        j += 1;
                        closed = true;
                        break;
                    }
                    _ => j += 1,
                }
            }
            let mut end = j.min(n);
            while !text.is_char_boundary(end) {
                end += 1;
            }
            // JSON-style keys ("name": …) read as keys
            let key = closed && (lang.name == "json") && text[end..].trim_start().starts_with(':');
            out.push((i, end, if key { Kind::Attr } else { Kind::String }));
            i = end;
            continue;
        }
        // Rust attributes: #[…] / #![…]
        if lang.hash_attrs && c == '#' && (at(i, "#[") || at(i, "#![")) {
            let e = text[i..].find(']').map_or(n, |e| i + e + 1);
            out.push((i, e, Kind::Attr));
            i = e;
            continue;
        }
        // C preprocessor lines (#include, #define …)
        if c == '#' && lang.name == "c" && text[..i].trim().is_empty() {
            let e = text[i + 1..].find(|c: char| !c.is_alphabetic()).map_or(n, |e| i + 1 + e);
            out.push((i, e, Kind::Keyword));
            i = e;
            continue;
        }
        if lang.at_attrs && c == '@' && text[i + 1..].starts_with(ident_start) {
            let e = text[i + 1..].find(|c: char| !(ident_char(c) || c == '.' || c == '-')).map_or(n, |e| i + 1 + e);
            out.push((i, e, Kind::Attr));
            i = e;
            continue;
        }
        if lang.dollar_vars && c == '$' {
            let rest = &text[i + 1..];
            let e = if rest.starts_with('{') {
                rest.find('}').map_or(n, |e| i + 1 + e + 1)
            } else if rest.starts_with('(') {
                i + 1
            } else {
                rest.find(|c: char| !ident_char(c)).map_or(n, |e| i + 1 + e)
            };
            if e > i + 1 {
                out.push((i, e, Kind::Attr));
                i = e;
                continue;
            }
        }
        if c.is_ascii_digit() && (i == 0 || !ident_char(text[..i].chars().next_back().unwrap_or(' '))) {
            let e = text[i..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).map_or(n, |e| i + e);
            // `1..10` (a Rust range) stops at the dots
            let e = text[i..e].find("..").map_or(e, |d| i + d);
            out.push((i, e, Kind::Number));
            i = e;
            continue;
        }
        if ident_start(c) || (c == '#' && lang.name == "css") {
            let s = i;
            let e = text[i + cl..].find(|c: char| !(ident_char(c) || (lang.name == "css" && c == '-') || (lang.name == "ruby" && (c == '?' || c == '!')))).map_or(n, |e| i + cl + e);
            let word = &text[s..e];
            let after = text[e..].trim_start();
            let is = |list: &[&str]| if lang.any_case { list.iter().any(|k| k.eq_ignore_ascii_case(word)) } else { list.contains(&word) };
            let kind = if is(lang.keywords) {
                Some(Kind::Keyword)
            } else if is(lang.constants) {
                Some(Kind::Number)
            } else if is(lang.types) {
                Some(Kind::Type)
            } else if lang.bang_macros && text[e..].starts_with('!') && !text[e..].starts_with("!=") {
                out.push((s, e + 1, Kind::Function));
                i = e + 1;
                continue;
            } else if after.starts_with('(') {
                Some(Kind::Function)
            } else if lang.caps_types && word.starts_with(|c: char| c.is_uppercase()) && word.chars().any(|c| c.is_lowercase()) {
                Some(Kind::Type)
            } else {
                None
            };
            if let Some(k) = kind {
                out.push((s, e, k));
            }
            i = e;
            continue;
        }
        i += cl;
    }
    out
}

/// Colour of each byte of `text` (`default` where no token).
pub fn colors(spans: &[Span], len: usize, default: Color32) -> Vec<Color32> {
    let mut v = vec![default; len];
    for &(s, e, k) in spans {
        let c = k.color();
        for x in v.iter_mut().take(e.min(len)).skip(s) {
            *x = c;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(path: &str, lines: &[&str]) -> Vec<Vec<(String, Kind)>> {
        let lang = for_path(path).unwrap();
        let mut st = State::Normal;
        lines.iter().map(|l| line(lang, l, &mut st).into_iter().map(|(s, e, k)| (l[s..e].to_string(), k)).collect()).collect()
    }

    fn has(v: &[(String, Kind)], text: &str, k: Kind) -> bool {
        v.iter().any(|(t, kk)| t == text && *kk == k)
    }

    #[test]
    fn rust() {
        let r = kinds("src/main.rs", &[r#"pub fn main() -> Result<(), String> { let x = 42; println!("hi {x}"); } // done"#]);
        let r = &r[0];
        assert!(has(r, "pub", Kind::Keyword));
        assert!(has(r, "fn", Kind::Keyword));
        assert!(has(r, "main", Kind::Function));
        assert!(has(r, "Result", Kind::Type));
        assert!(has(r, "42", Kind::Number));
        assert!(has(r, "println!", Kind::Function));
        assert!(has(r, r#""hi {x}""#, Kind::String));
        assert!(has(r, "// done", Kind::Comment));
        let l = kinds("a.rs", &["fn f<'a>(s: &'a str) -> char { 'x' }"]);
        assert!(has(&l[0], "'a", Kind::Attr));
        assert!(has(&l[0], "'x'", Kind::String));
    }

    #[test]
    fn block_comments_and_long_strings_span_lines() {
        let r = kinds("a.ts", &["const a = 1; /* start", "still comment", "end */ let b = `x", "y`;"]);
        assert!(has(&r[0], "/* start", Kind::Comment));
        assert!(has(&r[1], "still comment", Kind::Comment));
        assert!(has(&r[2], "end */", Kind::Comment));
        assert!(has(&r[2], "let", Kind::Keyword));
        assert!(has(&r[3], "y`", Kind::String));
        let p = kinds("a.py", &["def f():", "    \"\"\"Doc", "    more\"\"\"", "    return None  # x"]);
        assert!(has(&p[0], "def", Kind::Keyword));
        assert!(has(&p[2], "    more\"\"\"", Kind::String));
        assert!(has(&p[3], "None", Kind::Number));
        assert!(has(&p[3], "# x", Kind::Comment));
    }

    #[test]
    fn data_and_markup() {
        let y = kinds("ci.yml", &["  runs-on: ubuntu-latest  # x", "- name: Build"]);
        assert!(has(&y[0], "runs-on", Kind::Attr));
        assert!(has(&y[1], "name", Kind::Attr));
        let j = kinds("a.json", &[r#"{"name": "x", "ok": true}"#]);
        assert!(has(&j[0], r#""name""#, Kind::Attr));
        assert!(has(&j[0], r#""x""#, Kind::String));
        assert!(has(&j[0], "true", Kind::Number));
        let h = kinds("a.html", &[r#"<a href="/x">link</a> <!-- c -->"#]);
        assert!(has(&h[0], "a", Kind::Keyword));
        assert!(has(&h[0], "href", Kind::Attr));
        assert!(has(&h[0], r#""/x""#, Kind::String));
        assert!(!h[0].iter().any(|(t, _)| t == "link"));
        assert!(has(&h[0], "<!-- c -->", Kind::Comment));
        let s = kinds("x.sql", &["SELECT id FROM users -- all"]);
        assert!(has(&s[0], "SELECT", Kind::Keyword));
        assert!(has(&s[0], "-- all", Kind::Comment));
        let sh = kinds("run.sh", &[r#"echo "$HOME" ${X} # note"#]);
        assert!(has(&sh[0], "echo", Kind::Keyword));
        assert!(has(&sh[0], "# note", Kind::Comment));
    }

    #[test]
    fn multibyte_text_is_safe() {
        for path in ["a.rs", "a.ts", "a.py", "a.json", "a.html", "a.sh", "a.yml", "a.sql"] {
            let lang = for_path(path).unwrap();
            let mut st = State::Normal;
            for l in ["let s = \"\\한글\"; // 주석", "'\\é' \"é\\", "<a b=\"ü\">ö</a>", "k: \"日本\" # x", "$é ${ü} 'ö"] {
                for (s, e, _) in line(lang, l, &mut st) {
                    assert!(l.is_char_boundary(s) && l.is_char_boundary(e) && s <= e, "{path}: {l}");
                }
            }
        }
    }

    #[test]
    fn languages_by_name() {
        assert_eq!(for_path("Dockerfile").unwrap().name, "dockerfile");
        assert_eq!(for_path("web/App.TSX").unwrap().name, "javascript");
        assert!(for_path("README").is_none());
        assert_eq!(for_fence("rust ignore").unwrap().name, "rust");
        assert_eq!(for_fence("typescript").unwrap().name, "javascript");
        assert!(for_fence("text").is_none());
    }
}
