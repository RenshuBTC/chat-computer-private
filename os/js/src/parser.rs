//! A script's or module's text made a syntax tree (ast.rs), as ECMAScript 2024 says: every statement and
//! expression, classes (fields, private names, static blocks, accessors), generators and async functions, arrow
//! functions, destructuring (in declarations, parameters and assignments), spread and rest, template literals
//! (tagged too), optional chaining, `??`, logical assignment, BigInts, regular expressions (told apart from division
//! here, where the grammar says which), labels, `with` (outside strict code), and modules' import and export.
//! Semicolons are inserted where the language says they are (a line ended, a `}` comes, the text ends). Strict
//! code's rules are kept as far as they change what's parsed (its reserved words, octal literals, `with`).
//!
//! It reads a token at a time, looking ahead only to tell an arrow function's parameters from parentheses (to the
//! closing parenthesis, then whether `=>` follows), a `let` declaration from a name, and the like.

use crate::ast::*;
use crate::lexer::{LexState, Lexer, SyntaxError, Tok, Token, P};
use crate::names as n;
use crate::string::{Atom, Atoms, JsStr};
use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

type R<T> = Result<T, SyntaxError>;

/// What the code being parsed is inside of: what `yield`, `await`, `super`, `return`, `break` and `continue` may do
/// there, and whether it's strict.
#[derive(Clone, Copy, Default)]
struct Ctx {
    strict: bool,
    function: bool,
    generator: bool,
    is_async: bool,
    /// `super.x` may be used (a method's body), and `super()` (a derived class's constructor).
    super_prop: bool,
    super_call: bool,
    /// `new.target` may be used (any function but an arrow at the top).
    new_target: bool,
    module: bool,
    /// In a class body: private names may be used.
    class: bool,
    /// A class field's initializer, or a static block: `arguments` isn't allowed; `await` isn't in a static block.
    field: bool,
}

pub struct Parser<'a, 'b> {
    lx: Lexer<'a>,
    pub atoms: &'b mut Atoms,
    tok: Token,
    /// Where the token before the current one ended.
    prev_end: usize,
    ctx: Ctx,
    /// The labels around (and whether each labels a loop), and how deep in loops and switches it is.
    labels: Vec<(Atom, bool)>,
    loops: u32,
    switches: u32,
    /// The last property key read was written as a string ("constructor" isn't a class's constructor then).
    key_was_string: bool,
    /// A for's left side is being read: `{a = 1}` may be its pattern.
    cover_ok: bool,
    /// The assignment expression about to be read is an array's or object's element.
    literal_element: bool,
}

/// Parses a script.
pub fn parse_script(src: &str, atoms: &mut Atoms) -> R<Script> {
    parse(src, atoms, false, false)
}

/// Parses a module (strict, with import and export, `await` at the top).
pub fn parse_module(src: &str, atoms: &mut Atoms) -> R<Script> {
    parse(src, atoms, true, true)
}

/// Parses a script, strict from the start if `strict` (code that eval runs inside strict code).
pub fn parse(src: &str, atoms: &mut Atoms, module: bool, strict: bool) -> R<Script> {
    let mut p = Parser::new(src, atoms)?;
    p.ctx.module = module;
    p.ctx.strict = strict || module;
    p.ctx.is_async = module;
    let body = p.statements_until_end(true)?;
    Ok(Script { body, strict: p.ctx.strict, module })
}

/// Parses a function's parameters and body, as `new Function(params, body)` makes one (`kind`: "function",
/// "function*", "async function" or "async function*").
pub fn parse_function_parts(params: &str, body: &str, kind: &str, atoms: &mut Atoms) -> R<Rc<Func>> {
    let src = format!("({kind} anonymous({params}\n) {{\n{body}\n}})");
    // (The parameters and body checked apart first: neither can close the other early.)
    let mut p = Parser::new(&src, atoms)?;
    p.expect(P::LParen)?;
    let e = p.primary()?;
    p.expect(P::RParen)?;
    if p.tok.tok != Tok::Eof {
        return Err(p.err("the function's parts don't make one function"));
    }
    match e {
        Expr::Func(f) => Ok(f),
        _ => Err(SyntaxError { message: String::from("not a function"), at: 0 }),
    }
}

fn is_reserved(a: Atom) -> bool {
    a.0 <= n::WITH.0
}

fn is_strict_reserved(a: Atom) -> bool {
    matches!(a, n::YIELD | n::LET | n::STATIC | n::IMPLEMENTS | n::INTERFACE | n::PACKAGE | n::PRIVATE | n::PROTECTED | n::PUBLIC)
}

impl<'a, 'b> Parser<'a, 'b> {
    pub fn new(src: &'a str, atoms: &'b mut Atoms) -> R<Parser<'a, 'b>> {
        let mut lx = Lexer::new(src);
        let tok = lx.next(atoms)?;
        Ok(Parser { lx, atoms, tok, prev_end: 0, ctx: Ctx::default(), labels: Vec::new(), loops: 0, switches: 0, key_was_string: false, cover_ok: false, literal_element: false })
    }

    fn err(&self, message: &str) -> SyntaxError {
        SyntaxError { message: String::from(message), at: self.tok.start }
    }

    fn unexpected(&self) -> SyntaxError {
        let what = match &self.tok.tok {
            Tok::Eof => String::from("the end of the script"),
            Tok::Name(a, _) => format!("'{}'", self.atoms.get(*a)),
            Tok::Punct(_) | Tok::Private(_) => format!("'{}'", &self.lx.source()[self.tok.start..self.tok.end]),
            Tok::Number(_) | Tok::BigInt(_) => String::from("a number"),
            Tok::Str(..) => String::from("a string"),
            Tok::Template(..) => String::from("a template"),
            Tok::Regex(..) => String::from("a regular expression"),
        };
        self.err(&format!("Unexpected {what}"))
    }

    /// On to the next token.
    fn next(&mut self) -> R<Token> {
        self.prev_end = self.tok.end;
        let t = self.lx.next(self.atoms)?;
        Ok(core::mem::replace(&mut self.tok, t))
    }

    /// The token after the current one, without moving on.
    fn peek(&mut self) -> R<Token> {
        let s = self.lx.state();
        let t = self.lx.next(self.atoms);
        self.lx.restore(s);
        t
    }

    fn save(&self) -> (LexState, Token, usize) {
        (self.lx.state(), self.tok.clone(), self.prev_end)
    }

    fn back(&mut self, s: (LexState, Token, usize)) {
        self.lx.restore(s.0);
        self.tok = s.1;
        self.prev_end = s.2;
    }

    fn is(&self, p: P) -> bool {
        self.tok.tok == Tok::Punct(p)
    }

    fn eat(&mut self, p: P) -> R<bool> {
        if self.is(p) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, p: P) -> R<()> {
        if self.is(p) {
            self.next()?;
            Ok(())
        } else {
            Err(self.unexpected())
        }
    }

    /// The current token is the word `a` (not written with an escape).
    fn is_word(&self, a: Atom) -> bool {
        self.tok.tok == Tok::Name(a, false)
    }

    fn eat_word(&mut self, a: Atom) -> R<bool> {
        if self.is_word(a) {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect_word(&mut self, a: Atom) -> R<()> {
        if self.eat_word(a)? { Ok(()) } else { Err(self.unexpected()) }
    }

    /// `;`, or where one is inserted: before `}`, at the end, or after a line's end.
    fn semicolon(&mut self) -> R<()> {
        if self.eat(P::Semi)? {
            return Ok(());
        }
        if self.is(P::RBrace) || self.tok.tok == Tok::Eof || self.tok.newline {
            return Ok(());
        }
        Err(self.unexpected())
    }

    /// The current token is a name that can be an identifier here.
    fn is_ident(&self) -> bool {
        match &self.tok.tok {
            Tok::Name(a, _) => self.ident_ok(*a),
            _ => false,
        }
    }

    fn ident_ok(&self, a: Atom) -> bool {
        if is_reserved(a) {
            return false;
        }
        if self.ctx.strict && is_strict_reserved(a) {
            return false;
        }
        if a == n::YIELD && (self.ctx.generator || self.ctx.strict) {
            return false;
        }
        if a == n::AWAIT && (self.ctx.is_async || self.ctx.module || self.ctx.field) {
            return false;
        }
        true
    }

    /// An identifier (a binding's name, or a reference).
    fn ident(&mut self) -> R<Atom> {
        match self.tok.tok {
            Tok::Name(a, _) if self.ident_ok(a) => {
                if self.ctx.strict && matches!(a, n::EVAL | n::ARGUMENTS) {
                    // (Only binding them is an error; references are fine: the binding callers check.)
                }
                self.next()?;
                Ok(a)
            }
            _ => Err(self.unexpected()),
        }
    }

    /// A name a binding gives: an identifier, not eval or arguments in strict code.
    fn binding_ident(&mut self) -> R<Atom> {
        let at = self.tok.start;
        let a = self.ident()?;
        if self.ctx.strict && matches!(a, n::EVAL | n::ARGUMENTS) {
            return Err(SyntaxError { message: String::from("eval and arguments can't be bound in strict code"), at });
        }
        Ok(a)
    }

    /// Any name, keywords too (after `.`, as a property's name).
    fn any_name(&mut self) -> R<Atom> {
        match self.tok.tok {
            Tok::Name(a, _) => {
                self.next()?;
                Ok(a)
            }
            _ => Err(self.unexpected()),
        }
    }

    // ------------------------------------------------------------------------------------------------ statements

    /// Statements up to the end of the text (the top of a script or module), with its directives.
    fn statements_until_end(&mut self, top: bool) -> R<Vec<Stmt>> {
        let mut out = Vec::new();
        let mut prologue = true;
        while self.tok.tok != Tok::Eof {
            if prologue {
                prologue = self.directive()?;
            }
            out.push(if self.ctx.module && top { self.module_item()? } else { self.statement_item()? });
        }
        Ok(out)
    }

    /// A directive ("use strict") at the start of a body: makes the code strict. Whether more directives may follow.
    fn directive(&mut self) -> R<bool> {
        let Tok::Str(_, octal) = self.tok.tok else { return Ok(false) };
        let raw = &self.lx.source()[self.tok.start..self.tok.end];
        // (Only an expression statement of a string alone is a directive.)
        let after = self.peek()?;
        let alone = matches!(after.tok, Tok::Punct(P::Semi) | Tok::Punct(P::RBrace) | Tok::Eof) || after.newline && !matches!(after.tok, Tok::Punct(P::LParen | P::LBracket | P::Dot | P::Plus | P::Minus | P::Slash | P::Comma | P::Question) | Tok::Template(..));
        if !alone {
            return Ok(false);
        }
        if raw == "'use strict'" || raw == "\"use strict\"" {
            self.ctx.strict = true;
        }
        if octal && self.ctx.strict {
            return Err(self.err("an octal escape in strict code"));
        }
        Ok(true)
    }

    fn statement_item(&mut self) -> R<Stmt> {
        match self.tok.tok.clone() {
            Tok::Name(n::FUNCTION, false) => {
                let f = self.function(false, false, true)?;
                Ok(Stmt::Func(f))
            }
            Tok::Name(n::CLASS, false) => Ok(Stmt::Class(self.class(true)?)),
            Tok::Name(n::CONST, false) => self.lexical(VarKind::Const),
            Tok::Name(n::LET, false) if self.let_declaration()? => self.lexical(VarKind::Let),
            Tok::Name(n::ASYNC, false) if self.async_function_ahead()? => {
                self.next()?;
                Ok(Stmt::Func(self.function(true, false, true)?))
            }
            _ => self.statement(),
        }
    }

    /// `let` starts a declaration here (a name, `[` or `{` follows), rather than being a name itself.
    fn let_declaration(&mut self) -> R<bool> {
        let t = self.peek()?;
        Ok(match t.tok {
            Tok::Punct(P::LBracket) | Tok::Punct(P::LBrace) => true,
            Tok::Name(a, _) => !(t.newline && !self.ctx.strict && matches!(a, n::IN | n::INSTANCEOF | n::OF)) && (!is_reserved(a) || a == n::YIELD || a == n::AWAIT || a == n::LET),
            _ => false,
        })
    }

    /// `async` then `function` on the same line.
    fn async_function_ahead(&mut self) -> R<bool> {
        let t = self.peek()?;
        Ok(t.tok == Tok::Name(n::FUNCTION, false) && !t.newline)
    }

    fn lexical(&mut self, kind: VarKind) -> R<Stmt> {
        self.next()?;
        let decls = self.declarations(kind, false)?;
        self.semicolon()?;
        Ok(Stmt::Var(kind, decls))
    }

    /// Declarators: `a = 1, [b, c] = d`.
    fn declarations(&mut self, kind: VarKind, no_in: bool) -> R<Vec<Decl>> {
        let mut out = Vec::new();
        loop {
            let at = self.tok.start;
            let pat = self.binding_target()?;
            if kind != VarKind::Var && matches!(pat, Pat::Ident(n::LET)) {
                return Err(SyntaxError { message: String::from("let can't be a lexical binding's name"), at });
            }
            let init = if self.eat(P::Assign)? { Some(self.assign(no_in)?) } else { None };
            if init.is_none() && !no_in && (kind == VarKind::Const || !matches!(pat, Pat::Ident(_))) {
                return Err(self.err(if kind == VarKind::Const { "a const needs a value" } else { "a destructuring declaration needs a value" }));
            }
            out.push(Decl { pat, init });
            if !self.eat(P::Comma)? {
                return Ok(out);
            }
        }
    }

    fn statement(&mut self) -> R<Stmt> {
        match &self.tok.tok {
            Tok::Punct(P::LBrace) => Ok(Stmt::Block(self.block()?)),
            Tok::Punct(P::Semi) => {
                self.next()?;
                Ok(Stmt::Empty)
            }
            Tok::Name(a, false) => {
                let a = *a;
                match a {
                    n::VAR => {
                        self.next()?;
                        let d = self.declarations(VarKind::Var, false)?;
                        self.semicolon()?;
                        Ok(Stmt::Var(VarKind::Var, d))
                    }
                    n::IF => self.if_statement(),
                    n::FOR => self.for_statement(),
                    n::WHILE => {
                        self.next()?;
                        self.expect(P::LParen)?;
                        let test = self.expression(false)?;
                        self.close_paren_before_statement()?;
                        let body = self.loop_body()?;
                        Ok(Stmt::While(test, Box::new(body)))
                    }
                    n::DO => {
                        self.next()?;
                        let body = self.loop_body()?;
                        self.expect_word(n::WHILE)?;
                        self.expect(P::LParen)?;
                        let test = self.expression(false)?;
                        self.expect(P::RParen)?;
                        // (The semicolon after do-while is always optional.)
                        self.eat(P::Semi)?;
                        Ok(Stmt::DoWhile(Box::new(body), test))
                    }
                    n::RETURN => {
                        if !self.ctx.function {
                            return Err(self.err("return outside a function"));
                        }
                        self.next()?;
                        let value = if self.is(P::Semi) || self.is(P::RBrace) || self.tok.tok == Tok::Eof || self.tok.newline {
                            None
                        } else {
                            Some(self.expression(false)?)
                        };
                        self.semicolon()?;
                        Ok(Stmt::Return(value))
                    }
                    n::BREAK | n::CONTINUE => self.jump(a == n::BREAK),
                    n::THROW => {
                        self.next()?;
                        if self.tok.newline {
                            return Err(self.err("throw's value must be on its line"));
                        }
                        let e = self.expression(false)?;
                        self.semicolon()?;
                        Ok(Stmt::Throw(e))
                    }
                    n::TRY => self.try_statement(),
                    n::SWITCH => self.switch_statement(),
                    n::WITH => {
                        if self.ctx.strict {
                            return Err(self.err("with isn't allowed in strict code"));
                        }
                        self.next()?;
                        self.expect(P::LParen)?;
                        let obj = self.expression(false)?;
                        self.close_paren_before_statement()?;
                        let body = self.statement()?;
                        Ok(Stmt::With(obj, Box::new(body)))
                    }
                    n::DEBUGGER => {
                        self.next()?;
                        self.semicolon()?;
                        Ok(Stmt::Debugger)
                    }
                    n::FUNCTION => {
                        // (Annex B: a function declaration as an if's branch, in sloppy code.)
                        if self.ctx.strict {
                            return Err(self.err("a function declaration can't be a statement on its own in strict code"));
                        }
                        Ok(Stmt::Func(self.function(false, false, true)?))
                    }
                    n::CLASS | n::CONST => Err(self.err("a declaration can't be a statement on its own")),
                    n::IMPORT if !matches!(self.peek()?.tok, Tok::Punct(P::LParen) | Tok::Punct(P::Dot)) => {
                        Err(self.err("import declarations are only allowed in a module's top level"))
                    }
                    n::EXPORT => Err(self.err("export declarations are only allowed in a module's top level")),
                    _ => {
                        // A label?
                        if self.ident_ok(a) && self.peek()?.tok == Tok::Punct(P::Colon) {
                            return self.labeled();
                        }
                        self.expression_statement()
                    }
                }
            }
            _ => self.expression_statement(),
        }
    }

    fn expression_statement(&mut self) -> R<Stmt> {
        let e = self.expression(false)?;
        self.semicolon()?;
        Ok(Stmt::Expr(e))
    }

    /// The `)` before a statement (if's, while's, for's, with's): a regular expression may start the statement.
    fn close_paren_before_statement(&mut self) -> R<()> {
        if !self.is(P::RParen) {
            return Err(self.unexpected());
        }
        self.lx.set_regex_ok(true);
        self.next()?;
        Ok(())
    }

    fn block(&mut self) -> R<Vec<Stmt>> {
        self.expect(P::LBrace)?;
        let mut out = Vec::new();
        while !self.is(P::RBrace) {
            if self.tok.tok == Tok::Eof {
                return Err(self.unexpected());
            }
            out.push(self.statement_item()?);
        }
        // (After a block, a regular expression may start the next statement.)
        self.lx.set_regex_ok(true);
        self.next()?;
        Ok(out)
    }

    fn loop_body(&mut self) -> R<Stmt> {
        self.loops += 1;
        let s = self.statement();
        self.loops -= 1;
        s
    }

    fn labeled(&mut self) -> R<Stmt> {
        let label = self.ident()?;
        self.expect(P::Colon)?;
        if self.labels.iter().any(|(l, _)| *l == label) {
            return Err(self.err("a label is used twice"));
        }
        let is_loop = matches!(self.tok.tok, Tok::Name(n::FOR | n::WHILE | n::DO, false));
        self.labels.push((label, is_loop));
        let body = if self.is_word(n::FUNCTION) {
            if self.ctx.strict {
                return Err(self.err("a function can't be labelled in strict code"));
            }
            Ok(Stmt::Func(self.function(false, false, true)?))
        } else {
            self.statement()
        };
        self.labels.pop();
        Ok(Stmt::Labeled(label, Box::new(body?)))
    }

    fn jump(&mut self, is_break: bool) -> R<Stmt> {
        self.next()?;
        let label = if !self.tok.newline && self.is_ident() { Some(self.ident()?) } else { None };
        match label {
            Some(l) => match self.labels.iter().find(|(x, _)| *x == l) {
                None => return Err(self.err("a label that isn't there")),
                Some((_, is_loop)) if !is_break && !is_loop => return Err(self.err("continue to a label that isn't a loop's")),
                _ => {}
            },
            None if is_break && self.loops == 0 && self.switches == 0 => return Err(self.err("break outside a loop or switch")),
            None if !is_break && self.loops == 0 => return Err(self.err("continue outside a loop")),
            None => {}
        }
        self.semicolon()?;
        Ok(if is_break { Stmt::Break(label) } else { Stmt::Continue(label) })
    }

    fn if_statement(&mut self) -> R<Stmt> {
        self.next()?;
        self.expect(P::LParen)?;
        let test = self.expression(false)?;
        self.close_paren_before_statement()?;
        let then = self.statement()?;
        let otherwise = if self.eat_word(n::ELSE)? { Some(Box::new(self.statement()?)) } else { None };
        Ok(Stmt::If(test, Box::new(then), otherwise))
    }

    fn for_statement(&mut self) -> R<Stmt> {
        self.next()?;
        let is_await = if self.is_word(n::AWAIT) && (self.ctx.is_async || self.ctx.module) {
            self.next()?;
            true
        } else {
            false
        };
        self.expect(P::LParen)?;
        // The first part: a declaration, an expression, or nothing.
        let kind = match self.tok.tok.clone() {
            Tok::Name(n::VAR, false) => Some(VarKind::Var),
            Tok::Name(n::CONST, false) => Some(VarKind::Const),
            Tok::Name(n::LET, false) if self.let_declaration()? => Some(VarKind::Let),
            _ => None,
        };
        if let Some(kind) = kind {
            self.next()?;
            let pat = self.binding_target()?;
            if self.is_word(n::OF) || (self.is_word(n::IN) && !is_await) {
                let of = self.is_word(n::OF);
                self.next()?;
                let right = if of { self.assign(false)? } else { self.expression(false)? };
                self.close_paren_before_statement()?;
                let body = self.loop_body()?;
                let left = ForLeft::Var(kind, pat);
                return Ok(if of { Stmt::ForOf(left, right, Box::new(body), is_await) } else { Stmt::ForIn(left, right, Box::new(body)) });
            }
            // An ordinary for: the rest of the declarations.
            let init = if self.eat(P::Assign)? { Some(self.assign(true)?) } else { None };
            if init.is_none() && (kind == VarKind::Const || !matches!(pat, Pat::Ident(_))) {
                return Err(self.err("a declaration in a for needs a value"));
            }
            let mut decls = vec![Decl { pat, init }];
            if self.eat(P::Comma)? {
                decls.extend(self.declarations(kind, true)?);
            }
            return self.for_rest(Some(ForInit::Var(kind, decls)));
        }
        if self.is(P::Semi) {
            return self.for_rest(None);
        }
        // An expression, or a for-in/of's left side.
        let start_async = self.is_word(n::ASYNC);
        let start_let = self.is_word(n::LET);
        self.cover_ok = true;
        let e = self.expression_no_in();
        self.cover_ok = false;
        let e = e?;
        if !(self.is_word(n::OF) || self.is_word(n::IN)) && has_cover_init(&e) {
            return Err(self.err("Invalid shorthand property initializer"));
        }
        if self.is_word(n::OF) && !(start_async && matches!(e, Expr::Ident(n::ASYNC))) && !start_let || (self.is_word(n::IN) && !is_await) {
            let of = self.is_word(n::OF);
            self.next()?;
            let left = ForLeft::Pat(self.to_pattern(e)?);
            let right = if of { self.assign(false)? } else { self.expression(false)? };
            self.close_paren_before_statement()?;
            let body = self.loop_body()?;
            return Ok(if of { Stmt::ForOf(left, right, Box::new(body), is_await) } else { Stmt::ForIn(left, right, Box::new(body)) });
        }
        if is_await {
            return Err(self.err("for await needs of"));
        }
        self.for_rest(Some(ForInit::Expr(e)))
    }

    /// An expression where `in` isn't the operator (a for's first part).
    fn expression_no_in(&mut self) -> R<Expr> {
        self.expression(true)
    }

    fn for_rest(&mut self, init: Option<ForInit>) -> R<Stmt> {
        self.expect(P::Semi)?;
        let test = if self.is(P::Semi) { None } else { Some(self.expression(false)?) };
        self.expect(P::Semi)?;
        let update = if self.is(P::RParen) { None } else { Some(self.expression(false)?) };
        self.close_paren_before_statement()?;
        let body = self.loop_body()?;
        Ok(Stmt::For(init, test, update, Box::new(body)))
    }

    fn try_statement(&mut self) -> R<Stmt> {
        self.next()?;
        let block = self.block()?;
        let mut param = None;
        let mut handler = None;
        if self.eat_word(n::CATCH)? {
            if self.eat(P::LParen)? {
                param = Some(self.binding_target()?);
                self.expect(P::RParen)?;
            }
            handler = Some(self.block()?);
        }
        let finalizer = if self.eat_word(n::FINALLY)? { Some(self.block()?) } else { None };
        if handler.is_none() && finalizer.is_none() {
            return Err(self.err("try needs catch or finally"));
        }
        Ok(Stmt::Try(block, param, handler, finalizer))
    }

    fn switch_statement(&mut self) -> R<Stmt> {
        self.next()?;
        self.expect(P::LParen)?;
        let d = self.expression(false)?;
        self.expect(P::RParen)?;
        self.expect(P::LBrace)?;
        self.switches += 1;
        let mut cases = Vec::new();
        let mut seen_default = false;
        while !self.is(P::RBrace) {
            let test = if self.eat_word(n::CASE)? {
                Some(self.expression(false)?)
            } else if self.eat_word(n::DEFAULT)? {
                if seen_default {
                    return Err(self.err("a switch has two defaults"));
                }
                seen_default = true;
                None
            } else {
                return Err(self.unexpected());
            };
            self.expect(P::Colon)?;
            let mut body = Vec::new();
            while !self.is(P::RBrace) && !self.is_word(n::CASE) && !self.is_word(n::DEFAULT) {
                if self.tok.tok == Tok::Eof {
                    return Err(self.unexpected());
                }
                body.push(self.statement_item()?);
            }
            cases.push(Case { test, body });
        }
        self.switches -= 1;
        self.lx.set_regex_ok(true);
        self.next()?;
        Ok(Stmt::Switch(d, cases))
    }

    // ------------------------------------------------------------------------------------------------- modules

    fn module_item(&mut self) -> R<Stmt> {
        if self.is_word(n::IMPORT) && !matches!(self.peek()?.tok, Tok::Punct(P::LParen) | Tok::Punct(P::Dot)) {
            return self.import();
        }
        if self.is_word(n::EXPORT) {
            return self.export();
        }
        self.statement_item()
    }

    fn module_name(&mut self) -> R<JsStr> {
        match self.tok.tok.clone() {
            Tok::Str(s, _) => {
                self.next()?;
                Ok(s)
            }
            _ => Err(self.unexpected()),
        }
    }

    /// A name imported or exported: a name, or a string (`export { a as "b c" }`).
    fn export_name(&mut self) -> R<Atom> {
        match self.tok.tok.clone() {
            Tok::Str(s, _) => {
                self.next()?;
                Ok(self.atoms.intern(&s))
            }
            _ => self.any_name(),
        }
    }

    fn import(&mut self) -> R<Stmt> {
        self.next()?;
        let mut names = Vec::new();
        if let Tok::Str(..) = self.tok.tok {
            let from = self.module_name()?;
            self.with_clause()?;
            self.semicolon()?;
            return Ok(Stmt::Import(Import { from, names }));
        }
        if self.is_ident() {
            let local = self.binding_ident()?;
            names.push((Some(n::DEFAULT), local));
            if !self.eat(P::Comma)? {
                return self.import_from(names);
            }
        }
        if self.eat(P::Star)? {
            self.expect_word(n::AS)?;
            let local = self.binding_ident()?;
            names.push((None, local));
        } else if self.eat(P::LBrace)? {
            while !self.is(P::RBrace) {
                let is_string = matches!(self.tok.tok, Tok::Str(..));
                let imported = self.export_name()?;
                let local = if self.eat_word(n::AS)? {
                    self.binding_ident()?
                } else {
                    if is_string || !self.ident_ok(imported) {
                        return Err(self.err("an import of that name needs `as`"));
                    }
                    imported
                };
                names.push((Some(imported), local));
                if !self.eat(P::Comma)? {
                    break;
                }
            }
            self.expect(P::RBrace)?;
        } else {
            return Err(self.unexpected());
        }
        self.import_from(names)
    }

    fn import_from(&mut self, names: Vec<(Option<Atom>, Atom)>) -> R<Stmt> {
        self.expect_word(n::FROM)?;
        let from = self.module_name()?;
        self.with_clause()?;
        self.semicolon()?;
        Ok(Stmt::Import(Import { from, names }))
    }

    /// An import's attributes (`with { type: "json" }`), read and set aside.
    fn with_clause(&mut self) -> R<()> {
        let assert = self.atoms.intern_str("assert");
        if (self.is_word(n::WITH) || self.is_word(assert)) && !self.tok.newline {
            self.next()?;
            self.expect(P::LBrace)?;
            while !self.is(P::RBrace) {
                self.export_name()?;
                self.expect(P::Colon)?;
                self.module_name()?;
                if !self.eat(P::Comma)? {
                    break;
                }
            }
            self.expect(P::RBrace)?;
        }
        Ok(())
    }

    fn export(&mut self) -> R<Stmt> {
        self.next()?;
        if self.eat(P::Star)? {
            let alias = if self.eat_word(n::AS)? { Some(self.export_name()?) } else { None };
            self.expect_word(n::FROM)?;
            let from = self.module_name()?;
            self.with_clause()?;
            self.semicolon()?;
            return Ok(Stmt::Export(Export::All(alias, from)));
        }
        if self.eat_word(n::DEFAULT)? {
            if self.is_word(n::FUNCTION) || (self.is_word(n::ASYNC) && self.async_function_ahead()?) {
                let is_async = self.eat_word(n::ASYNC)?;
                let f = self.function(is_async, false, false)?;
                let name = f.name;
                return Ok(Stmt::Export(Export::Default(Box::new(Expr::Func(f)), name)));
            }
            if self.is_word(n::CLASS) {
                let c = self.class(false)?;
                let name = c.name;
                return Ok(Stmt::Export(Export::Default(Box::new(Expr::Class(c)), name)));
            }
            let e = self.assign(false)?;
            self.semicolon()?;
            return Ok(Stmt::Export(Export::Default(Box::new(e), None)));
        }
        if self.eat(P::LBrace)? {
            let mut names = Vec::new();
            while !self.is(P::RBrace) {
                let local = self.export_name()?;
                let exported = if self.eat_word(n::AS)? { self.export_name()? } else { local };
                names.push((local, exported));
                if !self.eat(P::Comma)? {
                    break;
                }
            }
            self.expect(P::RBrace)?;
            let from = if self.eat_word(n::FROM)? {
                let f = self.module_name()?;
                self.with_clause()?;
                Some(f)
            } else {
                None
            };
            self.semicolon()?;
            return Ok(Stmt::Export(Export::Names(names, from)));
        }
        // export var/let/const/function/class.
        let s = self.statement_item()?;
        match s {
            Stmt::Var(..) | Stmt::Func(_) | Stmt::Class(_) => Ok(Stmt::Export(Export::Decl(Box::new(s)))),
            _ => Err(self.err("export needs a declaration")),
        }
    }

    // ----------------------------------------------------------------------------------------------- functions

    /// A function, after `async` if it was async: `function`, a star if a generator, its name (needed for a
    /// declaration), its parameters and body.
    fn function(&mut self, is_async: bool, expression: bool, declaration: bool) -> R<Rc<Func>> {
        let start = if is_async { self.prev_start_of_async() } else { self.tok.start };
        self.expect_word(n::FUNCTION)?;
        let is_generator = self.eat(P::Star)?;
        // (A function expression's own name follows its own rules for yield and await.)
        let name = if self.is(P::LParen) {
            if declaration && !expression {
                return Err(self.err("a function declaration needs a name"));
            }
            None
        } else {
            let saved = self.ctx;
            if expression {
                self.ctx.generator = is_generator;
                self.ctx.is_async = is_async;
            }
            let r = self.binding_ident();
            self.ctx = saved;
            Some(r?)
        };
        self.function_rest(name, FuncKind::Normal, is_async, is_generator, start)
    }

    /// Where an `async` just before started (for a function's source text).
    fn prev_start_of_async(&self) -> usize {
        // (The async keyword ended where the token before ended; it's 5 letters.)
        self.prev_end.saturating_sub(5)
    }

    /// A function's parameters and body, from its `(`.
    fn function_rest(&mut self, name: Option<Atom>, kind: FuncKind, is_async: bool, is_generator: bool, start: usize) -> R<Rc<Func>> {
        let saved = self.ctx;
        let (labels, loops, switches) = (core::mem::take(&mut self.labels), self.loops, self.switches);
        self.loops = 0;
        self.switches = 0;
        self.ctx.function = true;
        self.ctx.generator = is_generator;
        self.ctx.is_async = is_async;
        self.ctx.new_target = true;
        self.ctx.field = false;
        match kind {
            FuncKind::Method | FuncKind::Getter | FuncKind::Setter => self.ctx.super_prop = true,
            FuncKind::Constructor { derived } => {
                self.ctx.super_prop = true;
                self.ctx.super_call = derived;
            }
            FuncKind::Normal => {
                self.ctx.super_prop = false;
                self.ctx.super_call = false;
            }
            _ => {}
        }
        let r = self.params_and_body(name, kind, is_async, is_generator, start);
        self.ctx = saved;
        self.labels = labels;
        self.loops = loops;
        self.switches = switches;
        r
    }

    fn params_and_body(&mut self, name: Option<Atom>, kind: FuncKind, is_async: bool, is_generator: bool, start: usize) -> R<Rc<Func>> {
        self.expect(P::LParen)?;
        let (params, rest) = self.formal_params(P::RParen)?;
        self.expect(P::RParen)?;
        match kind {
            FuncKind::Getter if !params.is_empty() || rest.is_some() => return Err(self.err("a getter takes no parameters")),
            FuncKind::Setter if params.len() != 1 || rest.is_some() => return Err(self.err("a setter takes one parameter")),
            _ => {}
        }
        let simple = rest.is_none() && params.iter().all(|p| p.default.is_none() && matches!(p.pat, Pat::Ident(_)));
        let was_strict = self.ctx.strict;
        let body = self.function_body()?;
        let strict = self.ctx.strict;
        if strict && !was_strict && !simple {
            return Err(self.err("\"use strict\" in a function whose parameters aren't simple"));
        }
        if strict {
            self.check_strict_params(&params, &rest, name, simple)?;
        }
        self.ctx.strict = was_strict;
        let end = self.prev_end;
        Ok(Rc::new(Func { name, params, rest, body: Body::Block(body), kind, is_async, is_generator, strict, simple, start, end, fields: None }))
    }

    /// Strict code's rules for parameters: none named eval or arguments, none twice.
    fn check_strict_params(&self, params: &[Param], rest: &Option<Pat>, name: Option<Atom>, _simple: bool) -> R<()> {
        let mut names = Vec::new();
        for p in params {
            bound_names(&p.pat, &mut names);
        }
        if let Some(r) = rest {
            bound_names(r, &mut names);
        }
        for (i, a) in names.iter().enumerate() {
            if matches!(*a, n::EVAL | n::ARGUMENTS) || names[..i].contains(a) {
                return Err(self.err("a parameter's name isn't allowed in strict code"));
            }
        }
        if matches!(name, Some(n::EVAL | n::ARGUMENTS)) {
            return Err(self.err("a function named eval or arguments in strict code"));
        }
        Ok(())
    }

    /// Parameters up to `close`: each a pattern with a default, the last perhaps a rest.
    fn formal_params(&mut self, close: P) -> R<(Vec<Param>, Option<Pat>)> {
        let mut params = Vec::new();
        let mut rest = None;
        while !self.is(close) {
            if self.eat(P::Ellipsis)? {
                rest = Some(self.binding_target()?);
                if self.is(P::Comma) {
                    return Err(self.err("a rest parameter must be the last"));
                }
                break;
            }
            let pat = self.binding_target()?;
            let default = if self.eat(P::Assign)? { Some(self.assign(false)?) } else { None };
            params.push(Param { pat, default });
            if !self.eat(P::Comma)? {
                break;
            }
        }
        Ok((params, rest))
    }

    /// `{ ... }`: a function's statements, with its directives.
    fn function_body(&mut self) -> R<Vec<Stmt>> {
        self.expect(P::LBrace)?;
        let mut out = Vec::new();
        let mut prologue = true;
        while !self.is(P::RBrace) {
            if self.tok.tok == Tok::Eof {
                return Err(self.unexpected());
            }
            if prologue {
                prologue = self.directive()?;
            }
            out.push(self.statement_item()?);
        }
        // (A function expression's `}`: division may follow; a declaration's, a statement. The caller knows; the
        // lexer's guess after `}` is division.)
        self.next()?;
        Ok(out)
    }

    /// An arrow function, its parameters at the current token (a name, or `(`).
    fn arrow(&mut self, is_async: bool, no_in: bool, start: usize) -> R<Expr> {
        let saved = self.ctx;
        let (params, rest) = if self.is(P::LParen) {
            self.next()?;
            // (An async arrow's parameters can't use await; a non-async's follow what's around it.)
            if is_async {
                self.ctx.is_async = true;
            }
            let r = self.formal_params(P::RParen);
            self.ctx = saved;
            let r = r?;
            self.expect(P::RParen)?;
            r
        } else {
            if is_async {
                self.ctx.is_async = true;
            }
            let a = self.binding_ident();
            self.ctx = saved;
            (vec![Param { pat: Pat::Ident(a?), default: None }], None)
        };
        if self.tok.newline {
            return Err(self.err("=> must be on its parameters' line"));
        }
        self.expect(P::Arrow)?;
        let simple = rest.is_none() && params.iter().all(|p| p.default.is_none() && matches!(p.pat, Pat::Ident(_)));
        let (labels, loops, switches) = (core::mem::take(&mut self.labels), self.loops, self.switches);
        self.loops = 0;
        self.switches = 0;
        self.ctx.is_async = is_async;
        self.ctx.generator = false;
        let was_strict = self.ctx.strict;
        let r = if self.is(P::LBrace) {
            self.ctx.function = true;
            self.function_body().map(Body::Block)
        } else {
            self.assign(no_in).map(|e| Body::Expr(Box::new(e)))
        };
        let strict = self.ctx.strict;
        self.ctx = saved;
        self.labels = labels;
        self.loops = loops;
        self.switches = switches;
        let body = r?;
        if strict && !was_strict && !simple {
            return Err(self.err("\"use strict\" in a function whose parameters aren't simple"));
        }
        if strict {
            self.check_strict_params(&params, &rest, None, simple)?;
        }
        let end = self.prev_end;
        Ok(Expr::Func(Rc::new(Func { name: None, params, rest, body, kind: FuncKind::Arrow, is_async, is_generator: false, strict, simple, start, end, fields: None })))
    }

    /// Whether arrow function parameters start here: `(` ... `)` then `=>` on the same line.
    fn arrow_ahead(&mut self) -> R<bool> {
        if !self.is(P::LParen) {
            return Ok(false);
        }
        let s = self.save();
        let mut depth = 0i32;
        let found = loop {
            match &self.tok.tok {
                Tok::Punct(P::LParen | P::LBracket | P::LBrace) => depth += 1,
                Tok::Punct(P::RParen | P::RBracket | P::RBrace) => {
                    depth -= 1;
                    if depth == 0 {
                        let t = self.next();
                        break match t {
                            Ok(_) => self.is(P::Arrow) && !self.tok.newline,
                            Err(_) => false,
                        };
                    }
                }
                Tok::Eof => break false,
                _ => {}
            }
            if self.next().is_err() {
                break false;
            }
        };
        self.back(s);
        Ok(found)
    }

    // ------------------------------------------------------------------------------------------------- classes

    fn class(&mut self, declaration: bool) -> R<Rc<Class>> {
        let start = self.tok.start;
        self.expect_word(n::CLASS)?;
        let saved = self.ctx;
        // (A class's code is strict, its name and heritage too.)
        self.ctx.strict = true;
        let name = if self.is_ident() && !self.is_word(n::EXTENDS) {
            Some(self.binding_ident()?)
        } else if declaration {
            self.ctx = saved;
            return Err(self.err("a class declaration needs a name"));
        } else {
            None
        };
        let extends = if self.eat_word(n::EXTENDS)? { Some(self.lhs(false)?) } else { None };
        let r = self.class_body(name, extends.is_some(), start);
        self.ctx = saved;
        let (constructor, members, end) = r?;
        Ok(Rc::new(Class { name, extends, constructor, members, start, end }))
    }

    fn class_body(&mut self, name: Option<Atom>, derived: bool, _start: usize) -> R<(Option<Rc<Func>>, Vec<Member>, usize)> {
        self.expect(P::LBrace)?;
        self.ctx.class = true;
        let mut constructor = None;
        let mut members = Vec::new();
        let _ = name;
        while !self.is(P::RBrace) {
            if self.eat(P::Semi)? {
                continue;
            }
            let member_start = self.tok.start;
            // `static` (a modifier, unless it's the member's name).
            let mut is_static = false;
            if self.is_word(n::STATIC) {
                let t = self.peek()?;
                if !matches!(t.tok, Tok::Punct(P::LParen | P::Assign | P::Semi | P::RBrace)) && !(t.newline && false) {
                    self.next()?;
                    is_static = true;
                    // A static block.
                    if self.is(P::LBrace) {
                        let saved = self.ctx;
                        self.ctx.function = false;
                        self.ctx.field = true;
                        self.ctx.is_async = false;
                        self.ctx.generator = false;
                        self.ctx.super_prop = true;
                        self.ctx.super_call = false;
                        self.ctx.new_target = true;
                        let (labels, loops, switches) = (core::mem::take(&mut self.labels), self.loops, self.switches);
                        self.loops = 0;
                        self.switches = 0;
                        let b = self.block();
                        self.ctx = saved;
                        self.labels = labels;
                        self.loops = loops;
                        self.switches = switches;
                        members.push(Member::StaticBlock(b?));
                        continue;
                    }
                }
            }
            let (is_async, is_generator, kind) = self.method_modifiers()?;
            let key = self.prop_key(true)?;
            if self.is(P::LParen) {
                // A method.
                let is_constructor = !is_static && matches!(key, PropKey::Name(n::CONSTRUCTOR)) && !self.key_was_string;
                let fkind = if is_constructor {
                    if is_async || is_generator || kind != MethodKind::Method {
                        return Err(self.err("a class's constructor can't be a getter, setter, generator or async"));
                    }
                    if constructor.is_some() {
                        return Err(self.err("a class has two constructors"));
                    }
                    FuncKind::Constructor { derived }
                } else {
                    match kind {
                        MethodKind::Get => FuncKind::Getter,
                        MethodKind::Set => FuncKind::Setter,
                        MethodKind::Method => FuncKind::Method,
                    }
                };
                if matches!(key, PropKey::Private(a) if self.atoms.get(a).is("constructor")) {
                    return Err(self.err("#constructor isn't allowed"));
                }
                if is_static && matches!(key, PropKey::Name(n::PROTOTYPE)) && !matches!(fkind, FuncKind::Constructor { .. }) {
                    return Err(self.err("a static method can't be named prototype"));
                }
                let fname = match &key {
                    PropKey::Name(a) => Some(*a),
                    _ => None,
                };
                let f = self.function_rest(fname, fkind, is_async, is_generator, member_start)?;
                if is_constructor {
                    constructor = Some(f);
                } else {
                    members.push(Member::Method { key, func: f, kind, is_static });
                }
                continue;
            }
            if is_async || is_generator || kind != MethodKind::Method {
                return Err(self.unexpected());
            }
            // A field.
            if matches!(key, PropKey::Name(n::CONSTRUCTOR)) || matches!(key, PropKey::Private(a) if self.atoms.get(a).is("constructor")) {
                return Err(self.err("a field can't be named constructor"));
            }
            if is_static && matches!(key, PropKey::Name(n::PROTOTYPE)) {
                return Err(self.err("a static field can't be named prototype"));
            }
            let value = if self.eat(P::Assign)? {
                let saved = self.ctx;
                self.ctx.field = true;
                self.ctx.function = false;
                self.ctx.is_async = false;
                self.ctx.generator = false;
                self.ctx.super_prop = true;
                self.ctx.super_call = false;
                self.ctx.new_target = true;
                let v = self.assign(false);
                self.ctx = saved;
                Some(v?)
            } else {
                None
            };
            self.semicolon()?;
            members.push(Member::Field { key, value, is_static });
        }
        let end = self.tok.end;
        self.next()?;
        Ok((constructor, members, end))
    }

    /// `async`, `*`, `get` or `set` before a method's name (each only when a name follows, rather than being the
    /// name itself).
    fn method_modifiers(&mut self) -> R<(bool, bool, MethodKind)> {
        let mut is_async = false;
        let mut kind = MethodKind::Method;
        if self.is_word(n::ASYNC) {
            let t = self.peek()?;
            if !t.newline && !matches!(t.tok, Tok::Punct(P::LParen | P::Assign | P::Semi | P::RBrace | P::Comma | P::Colon)) {
                self.next()?;
                is_async = true;
            }
        }
        let is_generator = self.eat(P::Star)?;
        if !is_async && !is_generator && (self.is_word(n::GET) || self.is_word(n::SET)) {
            let t = self.peek()?;
            if !matches!(t.tok, Tok::Punct(P::LParen | P::Assign | P::Semi | P::RBrace | P::Comma | P::Colon)) {
                kind = if self.is_word(n::GET) { MethodKind::Get } else { MethodKind::Set };
                self.next()?;
            }
        }
        Ok((is_async, is_generator, kind))
    }

    /// A property's key: a name (keywords too), a string, a number, `[computed]`, or (in a class) `#private`.
    fn prop_key(&mut self, class: bool) -> R<PropKey> {
        self.key_was_string = false;
        match self.tok.tok.clone() {
            Tok::Name(a, _) => {
                self.next()?;
                Ok(PropKey::Name(a))
            }
            Tok::Str(s, octal) => {
                if octal && self.ctx.strict {
                    return Err(self.err("an octal escape in strict code"));
                }
                self.next()?;
                self.key_was_string = true;
                Ok(PropKey::Name(self.atoms.intern(&s)))
            }
            Tok::Number(v) => {
                self.check_octal_number()?;
                self.next()?;
                Ok(PropKey::Num(v))
            }
            Tok::BigInt(digits) => {
                self.next()?;
                Ok(PropKey::Name(self.atoms.intern_str(&digits)))
            }
            Tok::Punct(P::LBracket) => {
                self.next()?;
                let e = self.assign(false)?;
                self.expect(P::RBracket)?;
                Ok(PropKey::Computed(Box::new(e)))
            }
            Tok::Private(a) if class => {
                self.next()?;
                Ok(PropKey::Private(a))
            }
            _ => Err(self.unexpected()),
        }
    }

    /// A legacy octal number (017) isn't allowed in strict code.
    fn check_octal_number(&self) -> R<()> {
        if self.ctx.strict {
            let raw = &self.lx.source()[self.tok.start..self.tok.end];
            let b = raw.as_bytes();
            if b.len() > 1 && b[0] == b'0' && b[1].is_ascii_digit() {
                return Err(self.err("a legacy octal number in strict code"));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------------------------------ patterns

    /// A binding's target: a name, or an array or object pattern.
    fn binding_target(&mut self) -> R<Pat> {
        match self.tok.tok {
            Tok::Punct(P::LBracket) => {
                self.next()?;
                let mut elems = Vec::new();
                let mut rest = None;
                loop {
                    if self.eat(P::RBracket)? {
                        break;
                    }
                    if self.eat(P::Comma)? {
                        elems.push(None);
                        continue;
                    }
                    if self.eat(P::Ellipsis)? {
                        rest = Some(Box::new(self.binding_target()?));
                        self.expect(P::RBracket)?;
                        break;
                    }
                    let pat = self.binding_target()?;
                    let default = if self.eat(P::Assign)? { Some(self.assign(false)?) } else { None };
                    elems.push(Some(PatElem { pat, default }));
                    if !self.is(P::RBracket) {
                        self.expect(P::Comma)?;
                    }
                }
                Ok(Pat::Array(elems, rest))
            }
            Tok::Punct(P::LBrace) => {
                self.next()?;
                let mut props = Vec::new();
                let mut rest = None;
                while !self.is(P::RBrace) {
                    if self.eat(P::Ellipsis)? {
                        rest = Some(Box::new(Pat::Ident(self.binding_ident()?)));
                        break;
                    }
                    let shorthand = matches!(self.tok.tok, Tok::Name(..));
                    let key_tok = self.tok.tok.clone();
                    let key = self.prop_key(false)?;
                    let (pat, default) = if self.eat(P::Colon)? {
                        let pat = self.binding_target()?;
                        let default = if self.eat(P::Assign)? { Some(self.assign(false)?) } else { None };
                        (pat, default)
                    } else {
                        let Tok::Name(a, _) = key_tok else { return Err(self.unexpected()) };
                        if !shorthand || !self.ident_ok(a) || (self.ctx.strict && matches!(a, n::EVAL | n::ARGUMENTS)) {
                            return Err(self.err("that name can't be bound"));
                        }
                        let default = if self.eat(P::Assign)? { Some(self.assign(false)?) } else { None };
                        (Pat::Ident(a), default)
                    };
                    props.push(PatProp { key, pat, default });
                    if !self.eat(P::Comma)? {
                        break;
                    }
                }
                self.expect(P::RBrace)?;
                Ok(Pat::Object(props, rest))
            }
            _ => Ok(Pat::Ident(self.binding_ident()?)),
        }
    }

    /// An expression parsed as one, made the assignment pattern it is (`[a, b] = ...`, `({a} = ...)`, `x.y = ...`).
    fn to_pattern(&self, e: Expr) -> R<Pat> {
        match e {
            Expr::Ident(a) => {
                if self.ctx.strict && matches!(a, n::EVAL | n::ARGUMENTS) {
                    return Err(self.err("eval and arguments can't be assigned to in strict code"));
                }
                Ok(Pat::Ident(a))
            }
            Expr::Member(_, _, false) | Expr::Index(_, _, false) | Expr::Private(_, _, false) | Expr::SuperMember(_) | Expr::SuperIndex(_) => {
                Ok(Pat::Expr(Box::new(e)))
            }
            Expr::Paren(inner) => match *inner {
                e @ (Expr::Ident(_) | Expr::Member(..) | Expr::Index(..) | Expr::Private(..) | Expr::SuperMember(_) | Expr::SuperIndex(_)) => {
                    self.to_pattern(e)
                }
                Expr::Paren(_) => self.to_pattern(*inner),
                _ => Err(self.err("that can't be assigned to")),
            },
            Expr::Array(elems) => {
                let mut out = Vec::new();
                let mut rest = None;
                let count = elems.len();
                for (i, el) in elems.into_iter().enumerate() {
                    match el {
                        ArrayElem::Hole => out.push(None),
                        ArrayElem::Item(e) => out.push(Some(self.to_elem(e)?)),
                        ArrayElem::Spread(e) => {
                            if i + 1 != count {
                                return Err(self.err("a rest element must be the last"));
                            }
                            if matches!(e, Expr::Assign(..)) {
                                return Err(self.err("a rest element can't have a default"));
                            }
                            rest = Some(Box::new(self.to_pattern(e)?));
                        }
                    }
                }
                Ok(Pat::Array(out, rest))
            }
            Expr::Object(props) => {
                let mut out = Vec::new();
                let mut rest = None;
                let count = props.len();
                for (i, p) in props.into_iter().enumerate() {
                    match p {
                        Prop::Init(key, value) => {
                            let PatElem { pat, default } = self.to_elem(value)?;
                            out.push(PatProp { key, pat, default });
                        }
                        Prop::Proto(value) => {
                            let PatElem { pat, default } = self.to_elem(value)?;
                            out.push(PatProp { key: PropKey::Name(n::PROTO), pat, default });
                        }
                        Prop::Short(a, default) => {
                            if self.ctx.strict && matches!(a, n::EVAL | n::ARGUMENTS) {
                                return Err(self.err("eval and arguments can't be assigned to in strict code"));
                            }
                            out.push(PatProp { key: PropKey::Name(a), pat: Pat::Ident(a), default });
                        }
                        Prop::Spread(e) => {
                            if i + 1 != count {
                                return Err(self.err("a rest property must be the last"));
                            }
                            let p = self.to_pattern(e)?;
                            if !matches!(p, Pat::Ident(_) | Pat::Expr(_)) {
                                return Err(self.err("a rest property must be a name or a property"));
                            }
                            rest = Some(Box::new(p));
                        }
                        Prop::Method(..) => return Err(self.err("a method can't be assigned to")),
                    }
                }
                Ok(Pat::Object(out, rest))
            }
            Expr::Assign(AssignOp::Assign, _, _) => Err(self.err("that can't be assigned to")),
            _ => Err(self.err("Invalid left-hand side in assignment")),
        }
    }

    /// An array's or object's element as a pattern's: `a = 1` is `a` with a default.
    fn to_elem(&self, e: Expr) -> R<PatElem> {
        match e {
            Expr::Assign(AssignOp::Assign, pat, default) => Ok(PatElem { pat: *pat, default: Some(*default) }),
            e => Ok(PatElem { pat: self.to_pattern(e)?, default: None }),
        }
    }

    // ----------------------------------------------------------------------------------------------- expressions

    /// An expression, commas and all.
    pub fn expression(&mut self, no_in: bool) -> R<Expr> {
        let first = self.assign(no_in)?;
        if !self.is(P::Comma) {
            return Ok(first);
        }
        let mut all = vec![first];
        while self.eat(P::Comma)? {
            all.push(self.assign(no_in)?);
        }
        Ok(Expr::Seq(all))
    }

    /// An assignment expression: arrow functions, `yield`, assignments, and what's below them.
    fn assign(&mut self, no_in: bool) -> R<Expr> {
        let start = self.tok.start;
        // Arrow functions.
        match self.tok.tok {
            Tok::Name(a, _) if self.ident_ok(a) || a == n::ASYNC => {
                let t = self.peek()?;
                if t.tok == Tok::Punct(P::Arrow) && !t.newline {
                    return self.arrow(false, no_in, start);
                }
                if a == n::ASYNC && !t.newline {
                    // async x => ..., async (x) => ...
                    if let Tok::Name(b, _) = t.tok
                        && b != n::FUNCTION
                    {
                        let s = self.save();
                        self.next()?;
                        if self.is_ident() && self.peek()?.tok == Tok::Punct(P::Arrow) {
                            let _ = b;
                            return self.arrow(true, no_in, start);
                        }
                        self.back(s);
                    }
                    if t.tok == Tok::Punct(P::LParen) {
                        let s = self.save();
                        self.next()?;
                        if self.arrow_ahead()? {
                            return self.arrow(true, no_in, start);
                        }
                        self.back(s);
                    }
                }
            }
            Tok::Punct(P::LParen) => {
                if self.arrow_ahead()? {
                    return self.arrow(false, no_in, start);
                }
            }
            _ => {}
        }
        if self.is_word(n::YIELD) && self.ctx.generator {
            return self.yield_expr(no_in);
        }
        let element = core::mem::replace(&mut self.literal_element, false);
        let left = self.conditional(no_in)?;
        // (`{a = 1}` is only a pattern: an error unless `=` follows, it's a for-in/of's left side, or it's an array's
        // or object's element, which the literal's own assignment looks at.)
        if !self.is(P::Assign) && !self.cover_ok && !element && has_cover_init(&left) {
            return Err(self.err("Invalid shorthand property initializer"));
        }
        let op = match self.tok.tok {
            Tok::Punct(p) => match p {
                P::Assign => AssignOp::Assign,
                P::PlusAssign => AssignOp::Op(BinOp::Add),
                P::MinusAssign => AssignOp::Op(BinOp::Sub),
                P::StarAssign => AssignOp::Op(BinOp::Mul),
                P::SlashAssign => AssignOp::Op(BinOp::Div),
                P::PercentAssign => AssignOp::Op(BinOp::Rem),
                P::StarStarAssign => AssignOp::Op(BinOp::Exp),
                P::ShlAssign => AssignOp::Op(BinOp::Shl),
                P::ShrAssign => AssignOp::Op(BinOp::Shr),
                P::UShrAssign => AssignOp::Op(BinOp::UShr),
                P::AmpAssign => AssignOp::Op(BinOp::BitAnd),
                P::PipeAssign => AssignOp::Op(BinOp::BitOr),
                P::CaretAssign => AssignOp::Op(BinOp::BitXor),
                P::AmpAmpAssign => AssignOp::Logical(LogOp::And),
                P::PipePipeAssign => AssignOp::Logical(LogOp::Or),
                P::QQAssign => AssignOp::Logical(LogOp::Coalesce),
                _ => return Ok(left),
            },
            _ => return Ok(left),
        };
        let target = if op == AssignOp::Assign {
            self.to_pattern(left)?
        } else {
            // A compound assignment's target is a name or a property, never a pattern.
            match self.to_pattern(left)? {
                p @ (Pat::Ident(_) | Pat::Expr(_)) => p,
                _ => return Err(self.err("Invalid left-hand side in assignment")),
            }
        };
        self.next()?;
        let value = self.assign(no_in)?;
        Ok(Expr::Assign(op, Box::new(target), Box::new(value)))
    }

    fn yield_expr(&mut self, no_in: bool) -> R<Expr> {
        if self.ctx.field {
            return Err(self.err("yield in a class field"));
        }
        self.next()?;
        if self.tok.newline {
            return Ok(Expr::Yield(None, false));
        }
        let delegate = self.eat(P::Star)?;
        let ends = matches!(self.tok.tok, Tok::Punct(P::RParen | P::RBracket | P::RBrace | P::Comma | P::Semi | P::Colon) | Tok::Eof)
            || (self.is_word(n::IN) && !delegate);
        if ends && !delegate {
            return Ok(Expr::Yield(None, false));
        }
        let arg = self.assign(no_in)?;
        Ok(Expr::Yield(Some(Box::new(arg)), delegate))
    }

    fn conditional(&mut self, no_in: bool) -> R<Expr> {
        let test = self.binary(0, no_in)?;
        if !self.eat(P::Question)? {
            return Ok(test);
        }
        // (`in` is the operator again between ? and :.)
        let then = self.assign(false)?;
        self.expect(P::Colon)?;
        let otherwise = self.assign(no_in)?;
        Ok(Expr::Cond(Box::new(test), Box::new(then), Box::new(otherwise)))
    }

    /// The binary operator at the current token, its precedence (higher binds tighter), if there's one.
    fn binary_op(&self, no_in: bool) -> Option<(u8, Result<BinOp, LogOp>)> {
        Some(match &self.tok.tok {
            Tok::Punct(p) => match p {
                P::QQ => (1, Err(LogOp::Coalesce)),
                P::PipePipe => (2, Err(LogOp::Or)),
                P::AmpAmp => (3, Err(LogOp::And)),
                P::Pipe => (4, Ok(BinOp::BitOr)),
                P::Caret => (5, Ok(BinOp::BitXor)),
                P::Amp => (6, Ok(BinOp::BitAnd)),
                P::Eq => (7, Ok(BinOp::Eq)),
                P::Ne => (7, Ok(BinOp::Ne)),
                P::StrictEq => (7, Ok(BinOp::StrictEq)),
                P::StrictNe => (7, Ok(BinOp::StrictNe)),
                P::Lt => (8, Ok(BinOp::Lt)),
                P::Gt => (8, Ok(BinOp::Gt)),
                P::Le => (8, Ok(BinOp::Le)),
                P::Ge => (8, Ok(BinOp::Ge)),
                P::Shl => (9, Ok(BinOp::Shl)),
                P::Shr => (9, Ok(BinOp::Shr)),
                P::UShr => (9, Ok(BinOp::UShr)),
                P::Plus => (10, Ok(BinOp::Add)),
                P::Minus => (10, Ok(BinOp::Sub)),
                P::Star => (11, Ok(BinOp::Mul)),
                P::Slash => (11, Ok(BinOp::Div)),
                P::Percent => (11, Ok(BinOp::Rem)),
                P::StarStar => (12, Ok(BinOp::Exp)),
                _ => return None,
            },
            Tok::Name(n::INSTANCEOF, false) => (8, Ok(BinOp::InstanceOf)),
            Tok::Name(n::IN, false) if !no_in => (8, Ok(BinOp::In)),
            _ => return None,
        })
    }

    /// Binary operators of precedence above `min`, by precedence climbing (`**` is right-associative; `??` can't be
    /// mixed with `||` or `&&` without parentheses).
    fn binary(&mut self, min: u8, no_in: bool) -> R<Expr> {
        // `#x in obj`.
        let mut left = if let Tok::Private(a) = self.tok.tok {
            if self.peek()?.tok != Tok::Name(n::IN, false) || no_in {
                return Err(self.unexpected());
            }
            self.next()?;
            self.next()?;
            let right = self.binary(8, no_in)?;
            Expr::PrivateIn(a, Box::new(right))
        } else {
            self.unary()?
        };
        while let Some((prec, op)) = self.binary_op(no_in) {
            if prec <= min && !(prec == 12 && min == 12) {
                break;
            }
            if prec == 12 && matches!(left, Expr::Unary(..) | Expr::Await(_)) {
                return Err(self.err("a unary operator's operand can't be raised with ** without parentheses"));
            }
            self.next()?;
            // (** binds to the right: its right side takes another ** at the same precedence.)
            let right = if prec == 12 { self.binary(11, no_in)? } else { self.binary(prec, no_in)? };
            left = match op {
                Ok(b) => Expr::Binary(b, Box::new(left), Box::new(right)),
                Err(l) => {
                    let mixes = |e: &Expr, with: LogOp| matches!(e, Expr::Logical(o, ..) if (*o == LogOp::Coalesce) != (with == LogOp::Coalesce));
                    if mixes(&left, l) || mixes(&right, l) {
                        return Err(self.err("?? can't be mixed with || or && without parentheses"));
                    }
                    Expr::Logical(l, Box::new(left), Box::new(right))
                }
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> R<Expr> {
        let op = match &self.tok.tok {
            Tok::Punct(P::Bang) => Some(UnOp::Not),
            Tok::Punct(P::Minus) => Some(UnOp::Neg),
            Tok::Punct(P::Plus) => Some(UnOp::Plus),
            Tok::Punct(P::Tilde) => Some(UnOp::BitNot),
            Tok::Name(n::TYPEOF, false) => Some(UnOp::TypeOf),
            Tok::Name(n::VOID, false) => Some(UnOp::Void),
            Tok::Name(n::DELETE, false) => Some(UnOp::Delete),
            _ => None,
        };
        if let Some(op) = op {
            self.next()?;
            let arg = self.unary()?;
            if op == UnOp::Delete && self.ctx.strict && matches!(strip_parens(&arg), Expr::Ident(_)) {
                return Err(self.err("delete of a name in strict code"));
            }
            if op == UnOp::Delete && matches!(strip_parens(&arg), Expr::Private(..)) {
                return Err(self.err("a private field can't be deleted"));
            }
            return Ok(Expr::Unary(op, Box::new(arg)));
        }
        if self.is(P::PlusPlus) || self.is(P::MinusMinus) {
            let add = self.is(P::PlusPlus);
            self.next()?;
            let arg = self.unary()?;
            self.check_update_target(&arg)?;
            return Ok(Expr::Update(Box::new(arg), add, true));
        }
        if self.is_word(n::AWAIT) && (self.ctx.is_async || (self.ctx.module && !self.ctx.function)) {
            if self.ctx.field {
                return Err(self.err("await in a class field or static block"));
            }
            self.next()?;
            let arg = self.unary()?;
            return Ok(Expr::Await(Box::new(arg)));
        }
        self.postfix()
    }

    fn check_update_target(&self, e: &Expr) -> R<()> {
        match strip_parens(e) {
            Expr::Ident(a) => {
                if self.ctx.strict && matches!(*a, n::EVAL | n::ARGUMENTS) {
                    return Err(self.err("eval and arguments can't be assigned to in strict code"));
                }
                Ok(())
            }
            Expr::Member(_, _, false) | Expr::Index(_, _, false) | Expr::Private(_, _, false) | Expr::SuperMember(_) | Expr::SuperIndex(_) => Ok(()),
            _ => Err(self.err("Invalid left-hand side expression in update operation")),
        }
    }

    fn postfix(&mut self) -> R<Expr> {
        let e = self.lhs(true)?;
        if (self.is(P::PlusPlus) || self.is(P::MinusMinus)) && !self.tok.newline {
            let add = self.is(P::PlusPlus);
            self.check_update_target(&e)?;
            self.next()?;
            return Ok(Expr::Update(Box::new(e), add, false));
        }
        Ok(e)
    }

    /// A left-hand-side expression: `new`, calls (if `calls`), property accesses, optional chains, tagged
    /// templates.
    fn lhs(&mut self, calls: bool) -> R<Expr> {
        let mut e = if self.is_word(n::NEW) {
            self.new_expr()?
        } else if self.is_word(n::SUPER) {
            self.super_expr()?
        } else if self.is_word(n::IMPORT) {
            self.import_expr()?
        } else {
            self.primary()?
        };
        let mut chained = false;
        loop {
            match &self.tok.tok {
                Tok::Punct(P::Dot) => {
                    self.next()?;
                    e = match self.tok.tok {
                        Tok::Private(a) => {
                            self.check_private()?;
                            self.next()?;
                            Expr::Private(Box::new(e), a, false)
                        }
                        _ => {
                            let a = self.any_name()?;
                            Expr::Member(Box::new(e), a, false)
                        }
                    };
                }
                Tok::Punct(P::QDot) => {
                    if !calls {
                        return Err(self.err("an optional chain can't be new's callee"));
                    }
                    self.next()?;
                    chained = true;
                    e = match self.tok.tok.clone() {
                        Tok::Punct(P::LParen) => {
                            let args = self.arguments()?;
                            Expr::Call(Box::new(e), args, true)
                        }
                        Tok::Punct(P::LBracket) => {
                            self.next()?;
                            let k = self.expression(false)?;
                            self.expect(P::RBracket)?;
                            Expr::Index(Box::new(e), Box::new(k), true)
                        }
                        Tok::Private(a) => {
                            self.check_private()?;
                            self.next()?;
                            Expr::Private(Box::new(e), a, true)
                        }
                        Tok::Template(..) => return Err(self.err("a template can't follow ?.")),
                        _ => {
                            let a = self.any_name()?;
                            Expr::Member(Box::new(e), a, true)
                        }
                    };
                }
                Tok::Punct(P::LBracket) => {
                    self.next()?;
                    let k = self.expression(false)?;
                    self.expect(P::RBracket)?;
                    e = Expr::Index(Box::new(e), Box::new(k), false);
                }
                Tok::Punct(P::LParen) if calls => {
                    let args = self.arguments()?;
                    e = Expr::Call(Box::new(e), args, false);
                }
                Tok::Template(.., true) => {
                    if chained {
                        return Err(self.err("a tagged template can't be in an optional chain"));
                    }
                    let (strings, exprs) = self.template(true)?;
                    e = Expr::Tagged(Box::new(e), strings, exprs);
                }
                _ => break,
            }
        }
        Ok(if chained { Expr::Chain(Box::new(e)) } else { e })
    }

    fn check_private(&self) -> R<()> {
        if !self.ctx.class {
            return Err(self.err("a private name outside a class"));
        }
        Ok(())
    }

    fn new_expr(&mut self) -> R<Expr> {
        let start = self.tok.start;
        self.next()?;
        if self.eat(P::Dot)? {
            if !self.eat_word(n::TARGET)? {
                return Err(self.unexpected());
            }
            if !self.ctx.new_target && !self.ctx.function && !self.ctx.field {
                return Err(SyntaxError { message: String::from("new.target outside a function"), at: start });
            }
            return Ok(Expr::NewTarget);
        }
        // The callee: a member expression (no calls), or another new.
        let callee = if self.is_word(n::NEW) {
            self.new_expr()?
        } else {
            let mut c = if self.is_word(n::SUPER) {
                self.super_expr()?
            } else if self.is_word(n::IMPORT) {
                return Err(self.err("import() can't be new's callee"));
            } else {
                self.primary()?
            };
            loop {
                match &self.tok.tok {
                    Tok::Punct(P::Dot) => {
                        self.next()?;
                        c = match self.tok.tok {
                            Tok::Private(a) => {
                                self.check_private()?;
                                self.next()?;
                                Expr::Private(Box::new(c), a, false)
                            }
                            _ => Expr::Member(Box::new(c), self.any_name()?, false),
                        };
                    }
                    Tok::Punct(P::LBracket) => {
                        self.next()?;
                        let k = self.expression(false)?;
                        self.expect(P::RBracket)?;
                        c = Expr::Index(Box::new(c), Box::new(k), false);
                    }
                    Tok::Template(.., true) => {
                        let (strings, exprs) = self.template(true)?;
                        c = Expr::Tagged(Box::new(c), strings, exprs);
                    }
                    Tok::Punct(P::QDot) => return Err(self.err("an optional chain can't be new's callee")),
                    _ => break,
                }
            }
            c
        };
        let args = if self.is(P::LParen) { self.arguments()? } else { Vec::new() };
        Ok(Expr::New(Box::new(callee), args))
    }

    fn super_expr(&mut self) -> R<Expr> {
        self.next()?;
        match self.tok.tok {
            Tok::Punct(P::LParen) => {
                if !self.ctx.super_call {
                    return Err(self.err("super() outside a derived class's constructor"));
                }
                let args = self.arguments()?;
                Ok(Expr::SuperCall(args))
            }
            Tok::Punct(P::Dot) => {
                if !self.ctx.super_prop {
                    return Err(self.err("super outside a method"));
                }
                self.next()?;
                Ok(Expr::SuperMember(self.any_name()?))
            }
            Tok::Punct(P::LBracket) => {
                if !self.ctx.super_prop {
                    return Err(self.err("super outside a method"));
                }
                self.next()?;
                let k = self.expression(false)?;
                self.expect(P::RBracket)?;
                Ok(Expr::SuperIndex(Box::new(k)))
            }
            _ => Err(self.err("super must be called or have a property taken")),
        }
    }

    fn import_expr(&mut self) -> R<Expr> {
        self.next()?;
        if self.eat(P::Dot)? {
            if !self.eat_word(n::META)? {
                return Err(self.unexpected());
            }
            if !self.ctx.module {
                return Err(self.err("import.meta outside a module"));
            }
            return Ok(Expr::ImportMeta);
        }
        self.expect(P::LParen)?;
        let spec = self.assign(false)?;
        // (A second argument: the import's options, evaluated and set aside.)
        if self.eat(P::Comma)? && !self.is(P::RParen) {
            let opts = self.assign(false)?;
            self.eat(P::Comma)?;
            self.expect(P::RParen)?;
            return Ok(Expr::Seq(vec![Expr::ImportCall(Box::new(spec)), opts]).first_of_seq());
        }
        self.expect(P::RParen)?;
        Ok(Expr::ImportCall(Box::new(spec)))
    }

    /// `(a, ...b)`: a call's arguments.
    fn arguments(&mut self) -> R<Vec<Arg>> {
        self.expect(P::LParen)?;
        let mut out = Vec::new();
        while !self.is(P::RParen) {
            if self.eat(P::Ellipsis)? {
                out.push(Arg::Spread(self.assign(false)?));
            } else {
                out.push(Arg::Item(self.assign(false)?));
            }
            if !self.eat(P::Comma)? {
                break;
            }
        }
        self.expect(P::RParen)?;
        Ok(out)
    }

    /// A template's parts and substitutions, from its first part's token. Tagged: parts with invalid escapes have
    /// no value; else they're an error.
    fn template(&mut self, tagged: bool) -> R<(Rc<TemplateStrings>, Vec<Expr>)> {
        let mut cooked = Vec::new();
        let mut raw = Vec::new();
        let mut exprs = Vec::new();
        loop {
            let Tok::Template(c, r, tail, head) = self.tok.tok.clone() else { return Err(self.unexpected()) };
            if head != cooked.is_empty() {
                return Err(self.unexpected());
            }
            if c.is_none() && !tagged {
                return Err(self.err("a template has an escape that isn't valid"));
            }
            cooked.push(c);
            raw.push(r);
            if tail {
                self.next()?;
                break;
            }
            self.next()?;
            exprs.push(self.expression(false)?);
            // (The lexer read the `}` that ended the substitution as the next part.)
            if !matches!(self.tok.tok, Tok::Template(.., false)) {
                return Err(self.unexpected());
            }
        }
        Ok((Rc::new(TemplateStrings { cooked, raw }), exprs))
    }

    fn primary(&mut self) -> R<Expr> {
        let start = self.tok.start;
        match self.tok.tok.clone() {
            Tok::Number(v) => {
                self.check_octal_number()?;
                self.next()?;
                Ok(Expr::Num(v))
            }
            Tok::BigInt(d) => {
                self.next()?;
                Ok(Expr::BigInt(d))
            }
            Tok::Str(s, octal) => {
                if octal && self.ctx.strict {
                    return Err(self.err("an octal escape in strict code"));
                }
                self.next()?;
                Ok(Expr::Str(s))
            }
            Tok::Template(.., true) => {
                let (strings, exprs) = self.template(false)?;
                let cooked = strings.cooked.iter().map(|c| c.clone().unwrap_or_else(JsStr::empty)).collect();
                Ok(Expr::Template(cooked, exprs))
            }
            Tok::Regex(body, flags) => {
                for (i, c) in flags.char_indices() {
                    if !"dgimsuyv".contains(c) || flags[..i].contains(c) {
                        return Err(self.err("a regular expression's flags aren't right"));
                    }
                }
                self.next()?;
                Ok(Expr::Regex(body, flags))
            }
            Tok::Punct(P::LParen) => {
                self.next()?;
                let e = self.expression(false)?;
                self.expect(P::RParen)?;
                Ok(Expr::Paren(Box::new(e)))
            }
            Tok::Punct(P::LBracket) => self.array_literal(),
            Tok::Punct(P::LBrace) => self.object_literal(),
            Tok::Punct(P::Slash) | Tok::Punct(P::SlashAssign) => {
                // (Division where a regular expression should start: read it again as one.)
                self.lx.rewind(start, true);
                self.tok = self.lx.next(self.atoms)?;
                self.primary()
            }
            Tok::Name(a, escaped) => match a {
                n::THIS if !escaped => {
                    self.next()?;
                    Ok(Expr::This)
                }
                n::NULL if !escaped => {
                    self.next()?;
                    Ok(Expr::Null)
                }
                n::TRUE if !escaped => {
                    self.next()?;
                    Ok(Expr::Bool(true))
                }
                n::FALSE if !escaped => {
                    self.next()?;
                    Ok(Expr::Bool(false))
                }
                n::FUNCTION if !escaped => {
                    let f = self.function(false, true, false)?;
                    Ok(Expr::Func(f))
                }
                n::CLASS if !escaped => Ok(Expr::Class(self.class(false)?)),
                n::ASYNC if !escaped && self.async_function_ahead()? => {
                    self.next()?;
                    let f = self.function(true, true, false)?;
                    Ok(Expr::Func(f))
                }
                _ => {
                    if escaped && (is_reserved(a) || (self.ctx.strict && is_strict_reserved(a))) {
                        return Err(self.err("a keyword can't be written with escapes"));
                    }
                    if a == n::ARGUMENTS && self.ctx.field && !self.ctx.function {
                        return Err(self.err("arguments in a class field"));
                    }
                    let a = self.ident()?;
                    Ok(Expr::Ident(a))
                }
            },
            Tok::Private(_) => Err(self.unexpected()),
            _ => Err(self.unexpected()),
        }
    }

    fn array_literal(&mut self) -> R<Expr> {
        self.next()?;
        let mut elems = Vec::new();
        loop {
            if self.is(P::RBracket) {
                break;
            }
            if self.eat(P::Comma)? {
                elems.push(ArrayElem::Hole);
                continue;
            }
            let spread = self.eat(P::Ellipsis)?;
            self.literal_element = true;
            let v = self.assign(false)?;
            let el = if spread { ArrayElem::Spread(v) } else { ArrayElem::Item(v) };
            elems.push(el);
            if !self.is(P::RBracket) {
                self.expect(P::Comma)?;
            }
        }
        self.next()?;
        Ok(Expr::Array(elems))
    }

    fn object_literal(&mut self) -> R<Expr> {
        self.next()?;
        let mut props = Vec::new();
        let mut proto_set = false;
        while !self.is(P::RBrace) {
            if self.eat(P::Ellipsis)? {
                props.push(Prop::Spread(self.assign(false)?));
            } else {
                let start = self.tok.start;
                let (is_async, is_generator, kind) = self.method_modifiers()?;
                let key_tok = self.tok.tok.clone();
                let key = self.prop_key(false)?;
                if self.is(P::LParen) {
                    let fkind = match kind {
                        MethodKind::Get => FuncKind::Getter,
                        MethodKind::Set => FuncKind::Setter,
                        MethodKind::Method => FuncKind::Method,
                    };
                    let fname = match &key {
                        PropKey::Name(a) => Some(*a),
                        _ => None,
                    };
                    let f = self.function_rest(fname, fkind, is_async, is_generator, start)?;
                    props.push(Prop::Method(key, f, kind));
                } else if is_async || is_generator || kind != MethodKind::Method {
                    return Err(self.unexpected());
                } else if self.eat(P::Colon)? {
                    self.literal_element = true;
                    let value = self.assign(false)?;
                    if matches!(key, PropKey::Name(n::PROTO)) && !matches!(key_tok, Tok::Punct(P::LBracket)) {
                        if proto_set {
                            // (Two __proto__s: an error, unless this turns out to be a pattern; kept as Init then.)
                            props.push(Prop::Init(key, value));
                        } else {
                            proto_set = true;
                            props.push(Prop::Proto(value));
                        }
                    } else {
                        props.push(Prop::Init(key, value));
                    }
                } else {
                    // Shorthand: `{a}`, and (only in a pattern) `{a = 1}`.
                    let Tok::Name(a, escaped) = key_tok else { return Err(self.unexpected()) };
                    if !self.ident_ok(a) || (escaped && is_reserved(a)) {
                        return Err(self.err("that name can't be a shorthand property"));
                    }
                    let default = if self.eat(P::Assign)? { Some(self.assign(false)?) } else { None };
                    props.push(Prop::Short(a, default));
                }
            }
            if !self.eat(P::Comma)? {
                break;
            }
        }
        self.expect(P::RBrace)?;
        Ok(Expr::Object(props))
    }
}

impl Expr {
    /// A sequence's first expression (import()'s options set aside): the sequence's value is its last, so this one
    /// is moved last.
    fn first_of_seq(self) -> Expr {
        match self {
            Expr::Seq(mut v) if v.len() == 2 => {
                let first = v.remove(0);
                v.push(first);
                Expr::Seq(v)
            }
            e => e,
        }
    }
}

/// Whether an object literal in `e` (or nested in its arrays and objects) has `{a = 1}`, which only a pattern may.
fn has_cover_init(e: &Expr) -> bool {
    match e {
        Expr::Object(props) => props.iter().any(|p| match p {
            Prop::Short(_, Some(_)) => true,
            Prop::Init(_, v) | Prop::Proto(v) | Prop::Spread(v) => has_cover_init(v),
            _ => false,
        }),
        Expr::Array(elems) => elems.iter().any(|el| match el {
            ArrayElem::Item(v) | ArrayElem::Spread(v) => has_cover_init(v),
            ArrayElem::Hole => false,
        }),
        Expr::Paren(inner) => has_cover_init(inner),
        _ => false,
    }
}

/// The expression inside any parentheses.
fn strip_parens(e: &Expr) -> &Expr {
    match e {
        Expr::Paren(inner) => strip_parens(inner),
        e => e,
    }
}

/// The names a pattern binds.
pub fn bound_names(p: &Pat, out: &mut Vec<Atom>) {
    match p {
        Pat::Ident(a) => out.push(*a),
        Pat::Expr(_) => {}
        Pat::Array(elems, rest) => {
            for e in elems.iter().flatten() {
                bound_names(&e.pat, out);
            }
            if let Some(r) = rest {
                bound_names(r, out);
            }
        }
        Pat::Object(props, rest) => {
            for p in props {
                bound_names(&p.pat, out);
            }
            if let Some(r) = rest {
                bound_names(r, out);
            }
        }
    }
}

/// Line and column (from 1) of a place in the text, for errors.
pub fn line_col(src: &str, at: usize) -> (usize, usize) {
    let before = &src[..at.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rfind('\n').map_or(before.chars().count(), |i| before[i + 1..].chars().count()) + 1;
    (line, col)
}
