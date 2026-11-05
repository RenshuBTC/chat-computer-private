//! A script's syntax tree, as the parser makes it and the compiler reads it.

use crate::string::{Atom, JsStr};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug)]
pub struct Script {
    pub body: Vec<Stmt>,
    pub strict: bool,
    pub module: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FuncKind {
    Normal,
    Arrow,
    Method,
    Getter,
    Setter,
    /// A class's constructor (`derived`: its class extends another).
    Constructor { derived: bool },
    /// A class's field initializers, run as the instance is made (or the class's static ones and static blocks).
    Fields,
}

#[derive(Debug)]
pub struct Func {
    pub name: Option<Atom>,
    pub params: Vec<Param>,
    pub rest: Option<Pat>,
    pub body: Body,
    pub kind: FuncKind,
    pub is_async: bool,
    pub is_generator: bool,
    pub strict: bool,
    /// Its parameters are names only (no defaults, destructuring or rest).
    pub simple: bool,
    /// Where it is in the source (for Function.prototype.toString).
    pub start: usize,
    pub end: usize,
    /// For a class's constructor: the class's instance fields' initializer, run on `this` once it's made.
    pub fields: Option<Rc<Func>>,
}

#[derive(Debug)]
pub enum Body {
    Block(Vec<Stmt>),
    /// An arrow function's expression.
    Expr(Box<Expr>),
}

#[derive(Debug)]
pub struct Param {
    pub pat: Pat,
    pub default: Option<Expr>,
}

#[derive(Debug)]
pub struct Class {
    pub name: Option<Atom>,
    pub extends: Option<Expr>,
    pub constructor: Option<Rc<Func>>,
    pub members: Vec<Member>,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MethodKind {
    Method,
    Get,
    Set,
}

#[derive(Debug)]
pub enum Member {
    Method { key: PropKey, func: Rc<Func>, kind: MethodKind, is_static: bool },
    Field { key: PropKey, value: Option<Expr>, is_static: bool },
    StaticBlock(Vec<Stmt>),
}

#[derive(Debug)]
pub enum PropKey {
    Name(Atom),
    /// A number written as a key (made its canonical string as it's used).
    Num(f64),
    Computed(Box<Expr>),
    Private(Atom),
}

#[derive(Debug)]
pub enum Prop {
    Init(PropKey, Expr),
    /// `{a}` (and in a pattern, `{a = 1}`).
    Short(Atom, Option<Expr>),
    Method(PropKey, Rc<Func>, MethodKind),
    Spread(Expr),
    /// `__proto__: x` (sets the object's prototype).
    Proto(Expr),
}

#[derive(Debug)]
pub enum ArrayElem {
    Hole,
    Item(Expr),
    Spread(Expr),
}

#[derive(Debug)]
pub enum Arg {
    Item(Expr),
    Spread(Expr),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    TypeOf,
    Void,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Exp,
    Shl,
    Shr,
    UShr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    InstanceOf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogOp {
    And,
    Or,
    Coalesce,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssignOp {
    Assign,
    Op(BinOp),
    Logical(LogOp),
}

#[derive(Debug)]
pub struct TemplateStrings {
    pub cooked: Vec<Option<JsStr>>,
    pub raw: Vec<JsStr>,
}

#[derive(Debug)]
pub enum Expr {
    Num(f64),
    Str(JsStr),
    BigInt(String),
    Bool(bool),
    Null,
    Regex(JsStr, String),
    /// A template: its strings (one more than its expressions) and expressions.
    Template(Vec<JsStr>, Vec<Expr>),
    Tagged(Box<Expr>, Rc<TemplateStrings>, Vec<Expr>),
    Ident(Atom),
    This,
    NewTarget,
    ImportMeta,
    Array(Vec<ArrayElem>),
    Object(Vec<Prop>),
    Func(Rc<Func>),
    Class(Rc<Class>),
    Unary(UnOp, Box<Expr>),
    /// `++x`, `x--`...: the target, whether it adds, and whether it's prefix.
    Update(Box<Expr>, bool, bool),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(LogOp, Box<Expr>, Box<Expr>),
    Assign(AssignOp, Box<Pat>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Arg>, bool),
    New(Box<Expr>, Vec<Arg>),
    Member(Box<Expr>, Atom, bool),
    Index(Box<Expr>, Box<Expr>, bool),
    Private(Box<Expr>, Atom, bool),
    PrivateIn(Atom, Box<Expr>),
    SuperMember(Atom),
    SuperIndex(Box<Expr>),
    SuperCall(Vec<Arg>),
    /// An optional chain's whole: inside it, a `?.` that meets null or undefined makes the whole undefined.
    Chain(Box<Expr>),
    Seq(Vec<Expr>),
    Yield(Option<Box<Expr>>, bool),
    Await(Box<Expr>),
    ImportCall(Box<Expr>),
    /// `(x)`: kept until the parser knows whether it's an expression or a pattern.
    Paren(Box<Expr>),
}

#[derive(Debug)]
pub enum Pat {
    Ident(Atom),
    /// A member expression assigned to (in assignment patterns only).
    Expr(Box<Expr>),
    Array(Vec<Option<PatElem>>, Option<Box<Pat>>),
    Object(Vec<PatProp>, Option<Box<Pat>>),
}

#[derive(Debug)]
pub struct PatElem {
    pub pat: Pat,
    pub default: Option<Expr>,
}

#[derive(Debug)]
pub struct PatProp {
    pub key: PropKey,
    pub pat: Pat,
    pub default: Option<Expr>,
}

#[derive(Debug)]
pub struct Decl {
    pub pat: Pat,
    pub init: Option<Expr>,
}

#[derive(Debug)]
pub enum ForInit {
    Var(VarKind, Vec<Decl>),
    Expr(Expr),
}

#[derive(Debug)]
pub enum ForLeft {
    Var(VarKind, Pat),
    Pat(Pat),
}

#[derive(Debug)]
pub struct Case {
    pub test: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub enum Stmt {
    Var(VarKind, Vec<Decl>),
    Func(Rc<Func>),
    Class(Rc<Class>),
    Expr(Expr),
    Block(Vec<Stmt>),
    Empty,
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    For(Option<ForInit>, Option<Expr>, Option<Expr>, Box<Stmt>),
    ForIn(ForLeft, Expr, Box<Stmt>),
    ForOf(ForLeft, Expr, Box<Stmt>, bool),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Continue(Option<Atom>),
    Break(Option<Atom>),
    Return(Option<Expr>),
    With(Expr, Box<Stmt>),
    Switch(Expr, Vec<Case>),
    Labeled(Atom, Box<Stmt>),
    Throw(Expr),
    Try(Vec<Stmt>, Option<Pat>, Option<Vec<Stmt>>, Option<Vec<Stmt>>),
    Debugger,
    Import(Import),
    Export(Export),
}

#[derive(Debug)]
pub struct Import {
    pub from: JsStr,
    /// (imported name, local name); imported None: the namespace (`* as x`); Some("default") for a default.
    pub names: Vec<(Option<Atom>, Atom)>,
}

#[derive(Debug)]
pub enum Export {
    /// `export var/let/const/function/class ...`.
    Decl(Box<Stmt>),
    /// `export default <expression>` (a function or class declared too, under its name if it has one).
    Default(Box<Expr>, Option<Atom>),
    /// `export { a as b, c } [from "m"]`.
    Names(Vec<(Atom, Atom)>, Option<JsStr>),
    /// `export * [as x] from "m"`.
    All(Option<Atom>, JsStr),
}
