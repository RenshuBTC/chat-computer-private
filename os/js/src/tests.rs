//! The engine's tests: scripts parsed, and run.

use crate::ast::*;
use crate::parser::{parse_module, parse_script};
use crate::string::Atoms;
use alloc::string::ToString;

fn parses(src: &str) -> Script {
    let mut atoms = Atoms::new();
    match parse_script(src, &mut atoms) {
        Ok(s) => s,
        Err(e) => panic!("{src:?} didn't parse: {} at {:?}", e.message, crate::parser::line_col(src, e.at)),
    }
}

fn fails(src: &str) {
    let mut atoms = Atoms::new();
    assert!(parse_script(src, &mut atoms).is_err(), "{src:?} parsed, but shouldn't");
}

#[test]
fn statements_and_expressions_parse() {
    for src in [
        "var a = 1, b; let [c, d = 2, ...e] = f; const {g, h: {i}, ...j} = k;",
        "if (a) b(); else if (c) d; else { e }",
        "for (let i = 0; i < 10; i++) {} for (const x of y) ; for (var k in o) ; for (;;) break;",
        "for (x of y) {} for (a.b in c) {} for ([a, b] of c) {} for await (const x of y) {}".replace("for await (const x of y) {}", "").as_str(),
        "while (x) { continue } do x(); while (y) label: for (;;) { break label }",
        "switch (x) { case 1: a(); break; default: b() }",
        "try { a() } catch (e) { b(e) } finally { c() } try {} catch {}",
        "function f(a, b = 2, ...c) { return a + b } function* g() { yield 1; yield* h() } async function k() { await x }",
        "async function* ag() { for await (const x of y) yield x }",
        "x = (a, b) => a + b; y = async (a) => { await a }; z = a => b => a * b; w = async a => a; v = () => ({})",
        "class A extends B { constructor() { super() } static s = 1; #p = 2; get x() { return this.#p } set x(v) {} *gen() {} async am() {} static { this.t = 3 } m() { return super.m() } }",
        "o = { a, b: 1, [c]: 2, get d() { return 1 }, set d(v) {}, e() {}, async f() {}, *g() {}, 'h': 3, 4: 5, __proto__: null, ...rest }",
        "a?.b?.[c]?.(d); a ?? b; a ||= b; a &&= c; a ??= d; a ** b ** c; (-a) ** b",
        "t = `x${a}y${`z${b}`}`; tag`a${b}c`; r = /ab+c/gi.test(s); n = 0x1f + 0o7 + 0b1 + 1_000 + 10n",
        "new A; new A(); new A.B(c); new new A()(); function q() { return new.target }",
        "a = b ? c : d; typeof a; void 0; delete a.b; !a; ~a; +a; -a; ++a; a--",
        "if (a) /re/.test(b); x = a / b / c; }".trim_end_matches(" }"),
        "with (o) { a }",
        "a\n++b",
        "let x; let\ny = 1",
        "({a, b} = c); [a, b] = [b, a]; ({a: x.y, b: [c]} = d)",
        "x = function () {} / 1",
        "label: { break label }",
        "var yield, let, static, await, async, of, get, set;",
        "async\nfunction f() {}",
        "a = { get: 1, set: 2, async: 3, static: 4 }; b = { get() {}, set() {}, async() {}, static() {} }",
        "class C { static async *m() {} static get [x]() {} 'constructor'() {} }",
        "x = a in b; for (var i = (a in b); i; ) ;",
        "a = /[/]/; b = /\\//g",
        "html: <!-- comment\n1",
    ] {
        parses(src);
    }
}

#[test]
fn errors_are_found() {
    for src in [
        "var 1a;",
        "a = ;",
        "function () {}",
        "class {}",
        "'use strict'; with (a) {}",
        "'use strict'; var let = 1",
        "'use strict'; delete x",
        "'use strict'; 017",
        "for (const x;;) {}",
        "a?.b = 1",
        "a ?? b || c",
        "-a ** b",
        "break;",
        "continue;",
        "return 1",
        "new.target",
        "super.x",
        "x = { a = 1 }",
        "(a, b) => { 'use strict'; }; f = (a = 1) => { 'use strict' }",
        "class A { constructor() {} constructor() {} }",
        "throw\n1",
        "let let = 1",
        "async () => { await: 1 }",
        "function* g() { var yield }",
    ] {
        fails(src);
    }
}

#[test]
fn the_tree_is_right() {
    let s = parses("a = b + c * d - e;");
    let Stmt::Expr(Expr::Assign(AssignOp::Assign, target, value)) = &s.body[0] else { panic!("{:?}", s.body) };
    assert!(matches!(**target, Pat::Ident(_)));
    // (b + c * d) - e
    let Expr::Binary(BinOp::Sub, left, _) = &**value else { panic!("{value:?}") };
    assert!(matches!(&**left, Expr::Binary(BinOp::Add, _, r) if matches!(**r, Expr::Binary(BinOp::Mul, ..))));
    let s = parses("'use strict'; x");
    assert!(s.strict);
    let s = parses("f = function () { 'use strict'; return 1 }");
    assert!(!s.strict);
    let s = parses("a ** b ** c");
    let Stmt::Expr(Expr::Binary(BinOp::Exp, _, r)) = &s.body[0] else { panic!() };
    assert!(matches!(**r, Expr::Binary(BinOp::Exp, ..)), "** is right-associative");
    let s = parses("a?.b.c");
    assert!(matches!(&s.body[0], Stmt::Expr(Expr::Chain(_))));
}

#[test]
fn modules_parse() {
    let mut atoms = Atoms::new();
    let m = parse_module(
        "import a, { b as c, d } from './x.js'; import * as ns from 'y'; import 'z'; export const e = 1; export default function () {} \
         export { a as f, c }; export * from 'w'; export * as g from 'v'; await x;",
        &mut atoms,
    );
    assert!(m.is_ok(), "{:?}", m.err().map(|e| e.message.to_string()));
}
