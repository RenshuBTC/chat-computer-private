# Chat Computer's JavaScript engine

The JavaScript engine of Chat Computer's own system (`os/`), for the web pages its own browser engine (`os/browser`)
shows: written from ECMAScript's specification (ECMA-262), with nothing from other engines and no crates, `no_std`
(it runs in Chat Computer's own kernel, and its tests run on Linux).

## How it works

* **Text to tree.** `lexer.rs` reads tokens (`string.rs`: JavaScript's UTF-16 strings, as ropes; names interned as
  atoms, `names.rs`'s at fixed numbers; `unicode_id.rs`: which characters names may have); `parser.rs` makes the
  syntax tree of `ast.rs`, the whole grammar.
* **Tree to bytecode.** `scope.rs` works out each function's scopes first: a variable no inner function uses lives
  in its frame's registers; one an inner function uses lives in its scope's environment on the heap (made as the
  scope is entered: a `let` in a loop gets a new one each time round). Names it can't resolve are the global
  object's (or, where `with` or a direct `eval` could change what a name means, are looked up by name as the code
  runs). Then `compiler.rs` compiles each function to `Op`s, a stack machine's instructions (`code.rs`); `break`,
  `continue` and `return` through `finally` blocks are worked out as it compiles.
* **Running.** `vm.rs` runs the bytecode with frames of its own: a call from JavaScript to JavaScript pushes a frame
  rather than calling a Rust function, so deep recursion is the VM's stack's, and a generator or async function is
  a frame set aside with its part of the stack and taken up again (`generators.rs`, which also has iteration).
  Exceptions unwind to the nearest handler (each frame's try blocks; `finally` runs with a completion record: what
  to do after it). Built-in functions are Rust functions (`builtins/`), called with the arguments where they are.
* **Values, objects and the heap.** `value.rs`: undefined, null, booleans, numbers, strings, symbols, objects and
  BigInts (`bigint.rs`); `convert.rs`: the conversions between them. Objects (`object.rs`) live in the heap, named
  by number; their properties are an ordered map, arrays' elements a vector, and `props.rs` has the operations on
  them (get, set, define, delete, and the exotic objects': arrays, strings, typed arrays, arguments, proxies). The
  garbage collector marks what the roots reach (the VM's stack and frames, the realm's objects, the jobs waiting,
  the host's handles) and sweeps the rest at calls, and only while no built-in function is running (so a built-in
  never has an object freed under it).
* **Built-ins.** `builtins/`: Object, Function, Error, Symbol, Reflect, Proxy and the global functions (`core.rs`),
  iterators and generators (`iter.rs`), Promise (`promise.rs`), Array (`array.rs`), Number, Math and BigInt
  (`number.rs`, with `math.rs`: Math's functions computed with no library, correctly rounded where it matters),
  JSON, String, Map, Set and the weak ones (`collections.rs`), RegExp (`regexp.rs`, on the backtracking matcher
  of `../regexp.rs` with `unicode_gc.rs`'s General_Category for `\p{…}`), Date (`date.rs`) and ArrayBuffer,
  DataView and the typed arrays (`typed.rs`).
* **Jobs.** Promise reactions are microtasks the VM runs when the host asks (after each script, event or timer).
* **Modules.** `module.rs`: a module is compiled into the code that makes its functions (run as it's linked) and its
  own code (an async function's, if it awaits at its top level); its top-level names live in its environment, and a
  name it imports is a cell there pointing at the exporting module's environment and slot, so imports are live.
  The host fetches modules (the VM says which addresses its loads want; a host with them at hand gives a function
  that reads them at once); linking resolves each import through re-exports and `export * from`, and evaluation
  follows ECMAScript's Cyclic Module Records, top-level await included. `import()` and `import.meta` too, and JSON
  modules (`with { type: "json" }`).

## Tests

`cargo test -p js` runs the engine's own tests. `examples/run.rs` runs a script (`print` and `console.log` print):
`cargo run --release -p js --example run -- FILE.js` (`-m FILE.js`: as a module, what it imports read from files).

`examples/test262_run.rs` runs Ecma's conformance tests (a copy of github.com/tc39/test262), each in a fresh
engine, a panic or a test that runs over 20 s counted as a failure; it is built with the `conformance` profile (a
release build whose panics unwind): `cargo build --profile conformance -p js --example test262_run`, then
`target/conformance/examples/test262_run PATH/TO/test262 test/language` (or `test/built-ins`, or a folder or file in
them; `--failures N` lists N failures, `--strict` runs each test in strict mode too; a module test's modules are
read from their files). Tests of features the engine doesn't have yet are skipped (the list is at the top of the
runner): deferred and source-phase imports, Intl, Temporal, SharedArrayBuffer and Atomics, and the newest
additions. On test262 of 4 November 2026 it passes 96.1% of test/language (22,077 of 22,972 run; all of
module-code, import and dynamic-import) and 94.5% of test/built-ins (16,199 of 17,147). What's left most: early errors
(programs it runs that it should refuse), `super` and `new.target` in `eval`'s code, the newest Unicode's
properties in regular expressions, and the Iterator helpers. `examples/test262.rs` checks the parser alone on the same tests.
