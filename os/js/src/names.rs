//! Names every atom table starts with, at fixed numbers: JavaScript's keywords and contextual words (for the parser),
//! and the property names the engine itself uses most (for the built-ins). `Atoms::new` interns them first, in this
//! order, so each constant here is its atom in every table.

use crate::string::Atom;

macro_rules! predefined {
    ($($name:ident = $text:literal,)*) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[repr(u32)]
        enum Index { $($name),* }
        $(pub const $name: Atom = Atom(Index::$name as u32);)*
        /// The names, in order.
        pub const TEXTS: &[&str] = &[$($text),*];
    };
}

predefined! {
    // Keywords.
    BREAK = "break",
    CASE = "case",
    CATCH = "catch",
    CLASS = "class",
    CONST = "const",
    CONTINUE = "continue",
    DEBUGGER = "debugger",
    DEFAULT = "default",
    DELETE = "delete",
    DO = "do",
    ELSE = "else",
    ENUM = "enum",
    EXPORT = "export",
    EXTENDS = "extends",
    FALSE = "false",
    FINALLY = "finally",
    FOR = "for",
    FUNCTION = "function",
    IF = "if",
    IMPORT = "import",
    IN = "in",
    INSTANCEOF = "instanceof",
    NEW = "new",
    NULL = "null",
    RETURN = "return",
    SUPER = "super",
    SWITCH = "switch",
    THIS = "this",
    THROW = "throw",
    TRUE = "true",
    TRY = "try",
    TYPEOF = "typeof",
    VAR = "var",
    VOID = "void",
    WHILE = "while",
    WITH = "with",
    // Reserved in strict code, or contextual.
    YIELD = "yield",
    LET = "let",
    STATIC = "static",
    IMPLEMENTS = "implements",
    INTERFACE = "interface",
    PACKAGE = "package",
    PRIVATE = "private",
    PROTECTED = "protected",
    PUBLIC = "public",
    AWAIT = "await",
    ASYNC = "async",
    OF = "of",
    GET = "get",
    SET = "set",
    TARGET = "target",
    META = "meta",
    FROM = "from",
    AS = "as",
    ARGUMENTS = "arguments",
    EVAL = "eval",
    UNDEFINED = "undefined",
    NAN = "NaN",
    INFINITY = "Infinity",
    // Properties.
    LENGTH = "length",
    PROTOTYPE = "prototype",
    CONSTRUCTOR = "constructor",
    NAME = "name",
    MESSAGE = "message",
    STACK = "stack",
    CAUSE = "cause",
    ERRORS = "errors",
    VALUE = "value",
    DONE = "done",
    NEXT = "next",
    WRITABLE = "writable",
    ENUMERABLE = "enumerable",
    CONFIGURABLE = "configurable",
    TO_STRING = "toString",
    VALUE_OF = "valueOf",
    TO_JSON = "toJSON",
    CALLEE = "callee",
    CALLER = "caller",
    THEN = "then",
    INDEX = "index",
    INPUT = "input",
    GROUPS = "groups",
    INDICES = "indices",
    LAST_INDEX = "lastIndex",
    SOURCE = "source",
    FLAGS = "flags",
    GLOBAL = "global",
    IGNORE_CASE = "ignoreCase",
    MULTILINE = "multiline",
    DOT_ALL = "dotAll",
    UNICODE = "unicode",
    UNICODE_SETS = "unicodeSets",
    STICKY = "sticky",
    HAS_INDICES = "hasIndices",
    EXEC = "exec",
    RAW = "raw",
    PROTO = "__proto__",
    GLOBAL_THIS = "globalThis",
    OBJECT = "Object",
    FUNCTION_ = "Function",
    ARRAY = "Array",
    STRING = "String",
    NUMBER = "Number",
    BOOLEAN = "Boolean",
    SYMBOL = "Symbol",
    BIGINT = "BigInt",
    ERROR = "Error",
    EMPTY = "",
    ANONYMOUS = "anonymous",
    DEFAULT_EXPORT = "*default*",
    ADD = "add",
    SIZE = "size",
    RESOLVE = "resolve",
    REJECT = "reject",
    STATUS = "status",
    REASON = "reason",
    FULFILLED = "fulfilled",
    REJECTED = "rejected",
    JOIN = "join",
    LAST = "last",
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_name_once() {
        for (i, a) in super::TEXTS.iter().enumerate() {
            assert!(!super::TEXTS[..i].contains(a), "{a} twice");
        }
    }
}
