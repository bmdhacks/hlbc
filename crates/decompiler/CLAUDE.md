# Decompiler CLAUDE.md

This document explains the hlbc-decompiler architecture and iteration workflow.

## CRITICAL: Do Not Modify Test Files

**NEVER modify the test files in `tests/roundtrip/src/` to make tests pass.** This is dishonest and has been a recurring problem across conversation compactions.

If a test fails:
1. **Fix the decompiler code**, not the test
2. If the fix is too complex, **say so honestly** - "This requires X which is hard because Y"
3. Document the limitation in Known Limitations below
4. Discuss with the user to find a solution together

The test files are intentionally read-only and owned by root to prevent this. If you find yourself able to edit them anyway, **don't**. The goal is a working decompiler, not passing tests.

## Philosophy

**Never crash, always produce readable output.** Even when control flow analysis fails, emit a comment like `// unhandled: JAlways +10` and continue. A complete imperfect decompilation is better than a partial crash.

## Architecture Overview

The decompiler transforms HashLink bytecode opcodes into an AST, then formats that AST as Haxe-like pseudocode.

```
Opcodes → DecompilerState → AST (Statement/Expr) → Formatted Output
```

### Key Files

| File | Purpose |
|------|---------|
| `src/lib.rs` | Main decompilation loop, opcode handlers, `DecompilerState` |
| `src/ast.rs` | AST types: `Statement`, `Expr`, `Constant` |
| `src/scopes.rs` | Scope stack for control flow (if/else, loops, switch, try/catch) |
| `src/fmt.rs` | AST → string formatting with indentation |
| `src/post.rs` | Post-processing visitors for AST cleanup |

### Core Data Structures

**DecompilerState** (`lib.rs`):
```rust
struct DecompilerState<'c> {
    f: &'c Function,           // Current function being decompiled
    reg_state: HashMap<Reg, Expr>,  // Current expression in each register
    expr_ctx: Vec<ExprCtx>,    // Context stack (constructors, etc.)
    scopes: Scopes,            // Control flow scope stack
    seen: HashSet<Str>,        // Declared variable names
    synthetic_var_counter: u32, // For generating v0, v1, etc.
}
```

**Scopes** (`scopes.rs`):
- Stack of `Scope` objects tracking nested control flow
- Each scope has: `ScopeType` (Len or Manual), `ScopeData` (If/Else/Loop/Switch/etc.), and accumulated `stmts`
- `advance()` decrements length-based scopes and closes them when they expire
- `push_*` methods create new scopes, `pop_*` methods close them

## Common Patterns

### Register State Tracking

The decompiler tracks what expression is "in" each register:
```rust
// After: OInt r5, 42
state.reg_state.insert(r5, Expr::Constant(Constant::Int(42)));

// After: OAdd r3, r1, r2
let left = state.expr(r1);   // Get current expr in r1
let right = state.expr(r2);  // Get current expr in r2
state.reg_state.insert(r3, Expr::Op(left, Op::Add, right));
```

### Synthetic Variables

When expressions get too complex (function calls, increments), create synthetic variables:
```rust
fn ensure_variable(&mut self, reg: Reg) -> Expr {
    // If already a variable, return it
    // Otherwise: create "v0", "v1", etc., emit assignment, update reg_state
}
```

### Loop Detection

Loops are detected by Label + backward JAlways:
```
Label           ; op N - loop start
...
JAlways -X      ; jumps back to op N
```

Loop conditions come from conditional jumps that exit the loop:
```
Label           ; loop start
JNull r0, +20   ; if r0 == null, exit loop (condition: r0 != null)
...
JAlways -15     ; back to label
```

### Control Flow Scope Lifecycle

1. **Open scope**: `push_if()`, `push_loop()`, `push_switch()`, etc.
2. **Accumulate statements**: opcodes add to `scopes.last_mut().stmts`
3. **Close scope**: `advance()` or explicit close, calls `make_stmt()` to convert to Statement

## Debugging Workflow

### Quick Iteration Cycle

```bash
# 1. Build
cargo build -p hlbc-decompiler --release

# 2. Test single function
./target/release/hlbc ../dead_cells.hl -c "decomp 30632"

# 3. Batch decompile and check
./target/release/hlbc ../dead_cells.hl --decompile-all -o ../analysis/decompiled/

# 4. Count issues
grep -r '\[missing expr\]' ../analysis/decompiled/ | wc -l
grep -r 'Decompilation failed' ../analysis/decompiled/ | wc -l
```

### Investigating Issues

**Find problematic functions:**
```bash
# Find files with specific issue
grep -l '\[missing expr\]' ../analysis/decompiled/**/*.hx

# Get function index from decompiled file header
head -5 ../analysis/decompiled/path/to/File.hx
# Shows: // fun@XXXXX (N ops)
```

**Examine bytecode:**
```bash
# Look up function in dump
grep -A 100 "=== Function 30632 ===" ../analysis/dead_cells_orig_dump.txt

# Or use CLI
./target/release/hlbc ../dead_cells.hl -c "f 30632"
```

**Trace execution:**
Add debug prints in `lib.rs`:
```rust
eprintln!("{i}: {:?} -> reg_state: {:?}", op, state.reg_state.keys());
```

### Common Issue Patterns

**`[missing expr]`** - Register has no tracked expression
- Usually: opcode handler not updating `reg_state`
- Or: constructor context eating the call without proper fallthrough
- Fix: ensure all opcodes that write to registers call `state.push_expr()` or update `reg_state`

**`[unknown X]`** - Type/variant not resolved
- Check the specific Unknown variant in `ast.rs`
- Usually needs lookup logic in the opcode handler

**Scope panics** - Control flow mismatch
- Add fallback handling instead of `unreachable!()`
- Emit comment and continue

## Testing Against Dead Cells

The primary test target is `../dead_cells.hl` (Dead Cells game bytecode).

**Regenerate all decompiled output:**
```bash
./target/release/hlbc ../dead_cells.hl --decompile-all -o ../analysis/decompiled/
```

**Check specific type:**
```bash
cat ../analysis/decompiled/h3d/impl/GlDriver.hx | head -100
```

**Metrics to track:**
```bash
# Issue counts (lower is better)
grep -r '\[missing expr\]' ../analysis/decompiled/ | wc -l
grep -r '\[unknown' ../analysis/decompiled/ | wc -l
grep -r 'Decompilation failed' ../analysis/decompiled/ | wc -l

# As of last session: 294 missing expr, 0 unknown, 0 failed
```

## Adding New Opcode Handlers

1. Find unhandled opcode in `lib.rs` main match
2. Add handler that:
   - Reads source registers with `state.expr(reg)`
   - Constructs appropriate `Expr` or `Statement`
   - Updates destination register: `state.push_expr(i, dst, expr)` or `state.reg_state.insert(dst, expr)`
   - Or pushes statement: `state.push_stmt(stmt)`

Example:
```rust
&Opcode::OSomething { dst, src1, src2 } => {
    let left = state.expr(src1);
    let right = state.expr(src2);
    state.push_expr(i, dst, Expr::Op(Box::new(left), Op::Something, Box::new(right)));
}
```

## Known Limitations

1. **Complex compound loop conditions**: Loops with `while (a || (b && c))` show as `while (true)` with internal breaks - acceptable tradeoff
2. **Exception handling**: Try/catch is basic, some edge cases produce `[missing expr]`
3. **Switch case merging**: Complex switch patterns may not reconstruct perfectly
4. **Closures**: Anonymous functions decompile but references may show as indices
5. **Array iteration**: `for (x in arr)` compiles to low-level iterator with `.bytes` access that can't be reconstructed. Use explicit while loops with indexing instead.

### Round-Trip Test Status (as of 2026-01-10)

**16/21 tests pass.** The remaining 5 failures have known limitations:

**Multi-class tests** (Inheritance, InterfaceTest, StaticMembers)
- These tests have multiple classes but the test infrastructure only extracts one class
- Need to extract all user classes from decompiled output, not just the main class

**PropertyAccess.hx** - Type coercion issue
- The decompiler outputs `Std.string(value)` in a context where `Int` is expected
- This is a pre-existing issue with property getter/setter decompilation

**DefaultParams.hx** - `hl.Ref` type parameter
- HashLink's `hl.Ref<T>` type needs proper generic parameter handling
- Currently outputs `hl.Ref` without the type parameter

## Code Quality Checklist

Before committing decompiler changes:
- [ ] `cargo build -p hlbc-decompiler --release` succeeds
- [ ] `cargo test -p hlbc-decompiler` passes
- [ ] Batch decompile produces 0 failed files
- [ ] Issue counts don't regress (check `[missing expr]`, etc.)
