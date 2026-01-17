# Decompiler CLAUDE.md

This document explains the hlbc-decompiler architecture and iteration workflow.

## CRITICAL: Do Not Modify Test Files

**NEVER modify the test files in `tests/roundtrip/src/` to make tests pass.** all failures are good indicators of bugs to fix

If a test fails:
1. **Fix the decompiler code**, not the test
2. If the fix is too complex, **say so honestly** - "This requires X which is hard because Y"
3. Document the limitation in Known Limitations below
4. Discuss with the user to find a solution together

The test files are intentionally read-only and owned by root to prevent this. If you find yourself able to edit them anyway, **don't**. The goal is a working decompiler, not passing tests.

## Philosophy

Asserting early is better than outputting incorrect code. Crashes are good, we will know to fix them.

## Architecture Overview

The decompiler uses a 7ish-pass pipeline to transform HashLink bytecode into Haxe source code:

```
Bytecode → Lifter → CFG → Analyzer → SSA Builder → Type Prop → Structurer → AST → Printer → Haxe
             (1)          (2)         (3)           (4)         (5)               (6)
```

| Pass | Module | Purpose |
|------|--------|---------|
| 1 | `lifter.rs` | Build petgraph CFG from bytecode (basic blocks + edges) |
| 2 | `analyzer.rs` | Compute dominator tree, identify natural loops, detect reducibility |
| 3 | `ssa.rs` | SSA conversion with φ-functions (Cytron et al. algorithm) |
| 4 | `type_prop.rs` | Forward type inference through SSA graph, φ-function type unification |
| 5 | `structurer.rs` | Convert SSA-CFG to structured AST, φ-elimination, expression inlining |
| 6 | `fmt.rs` | Format AST as Haxe pseudocode |

The pipeline is wired up in `lib.rs:decompile_code()`:
```rust
let cfg = Cfg::build(f);                           // Pass 1
let analysis = CfgAnalysis::analyze(&cfg);         // Pass 2
let ssa = SsaCfg::build(f, &cfg, &analysis);       // Pass 3
let type_info = TypePropagator::new(...).propagate(); // Pass 4
let stmts = Structurer::new(...).structure();      // Pass 5
// Pass 6 (fmt.rs) applied when rendering output
```

### Key Files

| File | Purpose |
|------|---------|
| `src/lib.rs` | Pipeline orchestration, entry points (`decompile_code`, `decompile_function`) |
| `src/lifter.rs` | Pass 1: CFG construction using petgraph |
| `src/analyzer.rs` | Pass 2: Dominator trees, natural loop detection |
| `src/ssa.rs` | Pass 3: SSA construction with φ-functions |
| `src/type_prop.rs` | Pass 4: Type inference through SSA graph |
| `src/structurer.rs` | Pass 5: Convert SSA-CFG to structured AST (~2500 lines) |
| `src/ast.rs` | AST types: `Statement`, `Expr`, `Constant`, `Operation` |
| `src/fmt.rs` | Pass 6: AST → Haxe string formatting |
| `src/post.rs` | Post-processing visitors (defined but not yet integrated) |
| `src/batch.rs` | Batch decompilation to files with package/class structure |
| `src/natives.rs` | Native function name lookup |

### Core Data Structures

**Cfg** (`lifter.rs`):
```rust
struct Cfg {
    graph: DiGraph<BasicBlock, EdgeKind>,  // petgraph directed graph
    entry: NodeIndex,
    op_to_block: HashMap<usize, NodeIndex>,
}
```

**CfgAnalysis** (`analyzer.rs`):
```rust
struct CfgAnalysis {
    dominators: Dominators<NodeIndex>,
    loops: Vec<NaturalLoop>,       // Header, body, back-edges, exits
    is_reducible: bool,
    node_to_loop: HashMap<NodeIndex, NodeIndex>,
}
```

**SsaCfg** (`ssa.rs`):
```rust
struct SsaCfg {
    blocks: HashMap<NodeIndex, SsaBlock>,  // φ-functions + SSA ops per block
    dom_frontiers: HashMap<NodeIndex, HashSet<NodeIndex>>,
}
```

**Note:** `post.rs` contains AST transformation visitors (IfExpressions, StringConcat, Trace, etc.) that are fully implemented but not yet wired into the pipeline. These could be integrated as a post-structuring cleanup pass.

## Key Algorithms

### Pass 1: CFG Construction (lifter.rs)

Splits bytecode into basic blocks at:
- Jump targets (Label opcodes, branch destinations)
- After unconditional jumps (JAlways, Ret, Throw)
- After conditional jumps (JSLt, JNull, etc.)

Edge types: `FallThrough`, `Jump`, `ConditionalTrue`, `ConditionalFalse`, `ExceptionHandler`

### Pass 2: Loop Detection (analyzer.rs)

Uses standard back-edge detection:
1. Compute dominator tree via `petgraph::algo::dominators`
2. Find back-edges: edges where target dominates source
3. Back-edge target is loop header, compute loop body via reverse reachability

### Pass 3: SSA Conversion (ssa.rs)

Implements Cytron et al. algorithm:
1. Compute dominance frontiers for each node
2. Insert φ-functions at merge points (DF nodes)
3. Rename variables: `reg0` → `v0_1`, `v0_2` with version counters
4. Track use-def chains for later inlining decisions

### Pass 5: Structuring (structurer.rs)

Reconstructs high-level control flow:
- Uses dominator tree to structure if/else (immediate dominator relationship)
- Uses loop info from analyzer for while loops
- φ-elimination: converts φ-functions to explicit assignments at branch ends
- Single-use inlining: expressions used once are inlined at use site

## Debugging Workflow

### Build Types

**IMPORTANT: `eprintln!()` output only appears in debug builds!**

```bash
cargo build                              # Debug build → ./target/debug/hlbc
cargo build --release                    # Release build → ./target/release/hlbc (NO eprintln!)
```

When adding debug prints, always test with the debug build:
```bash
cargo build && ./target/debug/hlbc file.hl -c "decomp 27"
```

### Quick Iteration Cycle

```bash
# 1. Build (use debug for eprintln, release for speed)
cargo build -p hlbc-decompiler

# 2. Test single function
./target/debug/hlbc ../dead_cells.hl -c "decomp 30632"

**Examine bytecode:**
```bash
# Look up function in dump
grep -A 100 "=== Function 30632 ===" ../analysis/dead_cells_orig_dump.txt

# Or use CLI
./target/debug/hlbc ../dead_cells.hl -c "f 30632"
```

**Trace execution:**
Add debug prints in `structurer.rs` or other modules:
```rust
eprintln!("{i}: {:?}", op);  // Only visible in debug builds!
```
Also you can use the gdb mcp, but it tends to fill up the context fast, so a better option is to ask the user to pilot GDB and get targeted information that way.

## Testing Against Hashlink Unit Tests

The primary test target is `/home/bmd/dev/hashlink/build/unit/unit.hl` (Hashlink unit test suite).

**Regenerate all decompiled output:**
```bash
./target/release/hlbc /home/bmd/dev/hashlink/build/unit/unit.hl --decompile-all -o ../analysis/unit_decompiled/
```

**Check specific type:**
```bash
cat ../analysis/unit_decompiled/h3d/impl/GlDriver.hx | head -100
```

**Metrics to track:**
```bash
# Issue counts (lower is better)
grep -r '\[missing expr\]' ../analysis/decompiled/ | wc -l
grep -r '\[unknown' ../analysis/decompiled/ | wc -l
grep -r 'Decompilation failed' ../analysis/decompiled/ | wc -l

# As of last session: 294 missing expr, 0 unknown, 0 failed
```

## Known Limitations

1. **Complex compound loop conditions**: Loops with `while (a || (b && c))` show as `while (true)` with internal breaks - acceptable tradeoff
2. **Switch case merging**: Complex switch patterns may not reconstruct perfectly
3. **Closures**: Anonymous functions decompile but references may show as indices
4. **Array iteration**: `for (x in arr)` compiles to low-level iterator with `.bytes` access that can't be reconstructed. Use explicit while loops with indexing instead.

### Round-Trip Test Status

**All tests pass** with the current SSA-based architecture.

Run tests with:
```bash
cd /home/bmd/hhg/hlbc/tests/roundtrip && ./run_tests.sh
```
