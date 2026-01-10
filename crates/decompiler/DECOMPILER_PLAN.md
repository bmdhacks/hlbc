# Decompiler Round-Trip Testing Plan: Toward the Identity Function

## Goal

Achieve **round-trip verification**: compile Haxe → decompile bytecode → recompile → compare. This proves the decompiler produces semantically correct output.

---

## Current State Analysis

### Decompiler Output (from exploration)

The decompiler produces **pseudo-Haxe that is NOT directly compilable**:

| Gap | Impact | Example |
|-----|--------|---------|
| No `package` declarations | BLOCKER | `class h3d.scene.Object {}` vs `package h3d.scene; class Object {}` |
| No `import` statements | BLOCKER | Types used without imports |
| Class names with dots | BLOCKER | `class h3d.scene.Object` is invalid Haxe |
| HL-specific types | BLOCKER | `hl.types.ArrayObj` instead of `Array<T>` |
| Synthetic variable names | Quality issue | `v0`, `v1` instead of meaningful names |
| Index annotations | Minor | `// fun@6121` comments (harmless) |

**Assessment**: ~30-40% of the way to compilable output.

### Haxe Unit Test Suite (from exploration)

Located at `/home/bmd/haxe_heaps_gles/haxe/tests/unit/src/`:
- **1,052 test files** with comprehensive language coverage
- Compiles to `bin/unit.hl` via `compile-hl.hxml`
- Tests compiled as **single suite**, not individually
- Uses `utest` framework, `Test` base class with assertion helpers

**Challenge**: Suite requires all tests together. Individual file compilation needs custom .hxml files.

---

## Proposed Approach

### Why Not Use Haxe Unit Tests Directly?

1. **Compilation dependency**: Tests require `utest` library and Test base class
2. **Macro complexity**: Many tests use compile-time macros
3. **Single-suite structure**: Not designed for individual file testing
4. **Complex patterns**: Tests include edge cases that may expose decompiler gaps before basics work

### Recommended Strategy: Minimal → Incremental

**Start with tiny, self-contained test files**, then expand:

```
Phase 1: Minimal Test Corpus (custom files)
    ↓
Phase 2: Make Decompiler Output Compilable
    ↓
Phase 3: Bytecode Comparison Tool
    ↓
Phase 4: Expand to Haxe Unit Tests (subset)
    ↓
Phase 5: Full Round-Trip CI
```

---

## Phase 1: Create Minimal Test Corpus

### Location

```
hlbc/tests/roundtrip/
├── src/
│   ├── basic/
│   │   ├── HelloWorld.hx      # Simplest possible
│   │   ├── Variables.hx       # Local vars, types
│   │   ├── Functions.hx       # Function calls, returns
│   │   └── Arithmetic.hx      # Operators
│   ├── control/
│   │   ├── IfElse.hx
│   │   ├── While.hx
│   │   ├── For.hx
│   │   └── Switch.hx
│   ├── oop/
│   │   ├── SimpleClass.hx
│   │   ├── Inheritance.hx
│   │   ├── Interfaces.hx
│   │   └── StaticMethods.hx
│   └── advanced/
│       ├── Arrays.hx
│       ├── Closures.hx
│       ├── TryCatch.hx
│       └── Enums.hx
├── compile.hxml               # Compile all to HL
└── run_roundtrip.sh          # Automation script
```

### Example Test File

```haxe
// tests/roundtrip/src/basic/Arithmetic.hx
package basic;

class Arithmetic {
    public static function main() {
        var a = 10;
        var b = 3;
        var sum = a + b;
        var diff = a - b;
        var prod = a * b;
        var quot = a / b;
        var mod = a % b;
        trace(sum);  // 13
    }
}
```

### Compile Script (compile.hxml)

```hxml
-cp src
--main basic.Arithmetic
-hl bin/arithmetic.hl
--debug

--next
-cp src
--main basic.Variables
-hl bin/variables.hl
--debug

# ... etc for each test
```

---

## Phase 2: Make Decompiler Output Compilable

### 2.1 Add Package Declarations

**Current output:**
```haxe
class basic.Arithmetic {
    // ...
}
```

**Required output:**
```haxe
package basic;

class Arithmetic {
    // ...
}
```

**Implementation**: In `batch.rs`, extract package from type name, emit `package X;` at file start.

### 2.2 Generate Import Statements

**Required**: Analyze type references in class, emit imports for external types.

```haxe
package h3d.scene;

import h3d.Matrix;
import h2d.Object;

class Renderer {
    // ...
}
```

**Implementation**: Track all type references during decompilation, emit imports in fmt.rs.

### 2.3 Fix Class Naming

**Current**: `class h3d.scene.Object`
**Required**: `class Object` (package already declared)

**Implementation**: In fmt.rs `Class::display()`, emit only the class name, not the full path.

### 2.4 Map HL Types to Haxe Types

| HL Type | Haxe Type |
|---------|-----------|
| `hl.types.ArrayObj` | `Array<Dynamic>` |
| `hl.Ref` | Use inline, or skip |
| `hl.UI8`, `hl.UI16` | `Int` |
| `hl.I64` | `haxe.Int64` |
| `hl.Bytes` | `haxe.io.Bytes` |

**Implementation**: Type mapping in fmt.rs `to_haxe_type()`.

### 2.5 Handle Synthetic Variables

Options:
1. **Keep as-is**: `v0`, `v1` are valid Haxe identifiers
2. **Use debug info**: Recover original names where available
3. **Smart naming**: Use type info (`obj0`, `arr1`, `str2`)

**For round-trip**: Option 1 is fine - names don't affect bytecode semantics.

---

## Phase 3: Bytecode Comparison Tool

### Challenge

Bytecode won't be byte-identical because:
- Different variable naming → different string pool
- Different expression ordering → different register allocation
- Compiler version differences

### Approaches

**Option A: Structural Comparison (Recommended)**

Compare normalized structures:
1. Parse both bytecodes
2. For each function, compare:
   - Opcode sequence (types, not operand indices)
   - Control flow graph structure
   - Type signatures
3. Report differences

**Option B: AST Comparison**

Decompile both bytecodes, compare ASTs:
1. Decompile original → AST1
2. Decompile recompiled → AST2
3. Normalize both (canonicalize variable names, sort statements)
4. Diff ASTs

**Option C: Execution Comparison**

Run both, compare output:
1. Both bytecodes should produce same trace() output
2. Simple but doesn't catch non-observable differences

### Proposed Tool

Add to hlbc-cli:

```bash
./target/release/hlbc --compare original.hl recompiled.hl
```

Output:
```
Comparing 45 functions...
Function basic.Arithmetic.main:
  Original:  15 ops, 3 locals
  Recompiled: 15 ops, 3 locals
  ✓ Structure matches

Function basic.Variables.test:
  Original:  8 ops
  Recompiled: 9 ops
  ✗ Op count differs
  Diff at op 5: OAdd vs OMul
```

---

## Phase 4: Expand to Haxe Unit Tests

Once basic round-trip works, extract tests from the Haxe suite:

### Step 1: Identify Self-Contained Tests

```bash
# Find tests that don't use macros or complex imports
grep -L "import.*macro" haxe/tests/unit/src/unit/*.hx
grep -L "@:build" haxe/tests/unit/src/unit/*.hx
```

### Step 2: Create Individual .hxml Files

For each suitable test:
```hxml
-cp src
-lib utest
--main unit.TestBasetypes
-hl bin/test_basetypes.hl
```

### Step 3: Add to Round-Trip Suite

---

## Phase 5: CI Integration

### Script: `run_roundtrip.sh`

```bash
#!/bin/bash
set -e

HAXE=/home/bmd/haxe_heaps_gles/haxe/haxe
HLBC=/home/bmd/haxe_heaps_gles/hlbc/target/release/hlbc

for hxml in tests/roundtrip/*.hxml; do
    name=$(basename "$hxml" .hxml)
    echo "=== Testing $name ==="

    # Step 1: Compile original
    $HAXE "$hxml"

    # Step 2: Decompile
    $HLBC "bin/${name}.hl" --decompile-all -o "tmp/${name}_decompiled/"

    # Step 3: Recompile decompiled output
    $HAXE -cp "tmp/${name}_decompiled" --main "${main}" -hl "bin/${name}_recompiled.hl"

    # Step 4: Compare
    $HLBC --compare "bin/${name}.hl" "bin/${name}_recompiled.hl"
done
```

---

## Implementation Order

| Order | Task | Effort | Files |
|-------|------|--------|-------|
| 1 | Create minimal test corpus (5-10 files) | Low | New: tests/roundtrip/ |
| 2 | Add package declarations to output | Low | batch.rs, fmt.rs |
| 3 | Fix class naming (remove FQN) | Low | fmt.rs |
| 4 | Generate import statements | Medium | fmt.rs, new analysis pass |
| 5 | Map HL types to Haxe types | Medium | fmt.rs |
| 6 | Build comparison tool | Medium | New: compare.rs or cli |
| 7 | Test and iterate on minimal corpus | Variable | All |
| 8 | Expand to Haxe unit tests | Variable | tests/roundtrip/ |

---

## Risk Assessment

### High Confidence
- Package declarations and class naming fixes are straightforward
- Minimal test corpus is easy to create
- Basic bytecode parsing for comparison exists

### Medium Confidence
- Import generation requires tracking type references
- Type mapping may have edge cases
- Some Haxe unit tests may use features we can't round-trip

### Known Limitations
- Macros can't be round-tripped (compile-time only)
- Inline functions may change structure
- Optimization differences between compiler versions
- Debug info may not survive round-trip

---

## Success Criteria

**Phase 1 Complete**: 10+ minimal test files compile
**Phase 2 Complete**: Decompiled output compiles without errors
**Phase 3 Complete**: Comparison tool reports structural matches
**Phase 4 Complete**: 50%+ of Haxe unit tests pass round-trip
**Phase 5 Complete**: CI catches regressions automatically

---

## Decisions (User Input)

1. **Comparison approach**: **Both** - Start with execution comparison (trace output), add bytecode structure comparison later for deeper verification.

2. **Test scope**: **Incremental** - Start with single-function files, expand to full classes as things work.

3. **Debug info**: **Semantic equivalence only** - Debug info preservation is not required; same behavior is sufficient.

4. **Compiler**: Use the local Haxe compiler at `/home/bmd/haxe_heaps_gles/haxe/haxe`

---

---

## Session Results (2026-01-08)

### Completed

1. **Created minimal test corpus**: 4 test files (HelloWorld, Arithmetic, Conditionals, Loop)

2. **Fixed decompiler output for compilable Haxe**:
   - Added `package` declarations to class output (`package haxe.iterators;`)
   - Fixed class naming (removed FQN dots, now `class ArrayKeyValueIterator` not `class haxe.iterators.ArrayKeyValueIterator`)
   - Created `HaxeFmt` formatter that doesn't add `@index` suffixes to type names
   - Fixed `Statement::Throw` missing semicolon
   - Mapped lowercase abstract types (like `hl_random`) to `Dynamic`
   - Increased INDENT buffer for deeply nested code

3. **Successful round-trips**:
   - `HelloWorld.hx` - simple trace output
   - `StringOnly.hx` - strings without int conversion
   - `Conditionals.hx` - if/else, ternary expressions

4. **Identified remaining issues**:
   - **Int-to-string conversion**: `itos(x,x)` + `__alloc__` not inlined back to expression
   - **Loop reconstruction**: `while (cond) {...}` becomes `while (true) { if (cond) {...} }`
   - **Variable naming in loops**: `v4++` instead of `count++`

### Metrics (Dead Cells)

- `[missing expr]`: 0
- `null.array`: 0
- `null.field` (actual, not string literals): 0

### Files Modified

| File | Changes |
|------|---------|
| `crates/decompiler/src/fmt.rs` | Added `HaxeFmt` formatter, package declarations, fixed class naming, throw semicolon |
| `crates/decompiler/src/batch.rs` | No changes needed |

### Next Steps

1. Fix int-to-string pattern inlining (statement-level pass needed)
2. Improve loop reconstruction
3. Fix loop variable naming

---

## Concrete First Steps

### Step 1: Create Minimal Test Files

Create 3-5 single-function test files to establish the workflow:

```
hlbc/tests/roundtrip/src/
├── HelloWorld.hx      # trace("hello")
├── Arithmetic.hx      # basic math operators
├── Conditionals.hx    # if/else
└── Loop.hx            # while loop
```

### Step 2: Compile Test Files

```bash
cd /home/bmd/haxe_heaps_gles/hlbc/tests/roundtrip
/home/bmd/haxe_heaps_gles/haxe/haxe -cp src --main HelloWorld -hl bin/hello.hl --debug
```

### Step 3: Decompile and Attempt Recompile

```bash
# Decompile
./target/release/hlbc bin/hello.hl --decompile-all -o tmp/decompiled/

# Attempt recompile (will fail initially - identifies what to fix)
/home/bmd/haxe_heaps_gles/haxe/haxe -cp tmp/decompiled --main HelloWorld -hl bin/hello_rt.hl
```

### Step 4: Fix Decompiler Output Issues

Iterate on decompiler fixes based on compilation errors:
1. Add package declaration
2. Fix class naming
3. Add imports
4. Fix type names

### Step 5: Compare Execution

```bash
# Run original
hl bin/hello.hl > /tmp/orig.txt

# Run round-tripped
hl bin/hello_rt.hl > /tmp/rt.txt

# Compare
diff /tmp/orig.txt /tmp/rt.txt
```

---

## Enum Pattern Matching Reconstruction

### Problem

The decompiler outputs low-level enum access patterns that are not valid Haxe:

```haxe
// Current (invalid)
switch (opt.constructorIndex) {
    case 0: return -1;
    case 1: v = opt.0; return v;
}

// Desired (valid Haxe)
switch (opt) {
    case None: return -1;
    case Some(v): return v;
}
```

### Bytecode Pattern

Enum pattern matching compiles to:
```
EnumIndex   reg1 = variant of reg0    // Extract constructor index
Switch { reg: Reg(1), offsets: [...] }
...
EnumField   reg2 = (reg0 as Some).0   // Extract field in case body
```

Key opcodes:
- `EnumIndex { dst, value }` - extracts variant tag (0, 1, 2...)
- `EnumField { dst, value, construct, field }` - extracts enum field
- `MakeEnum { dst, construct, args }` - creates enum value

### Implementation Strategy

**Approach: Post-processing pass to reconstruct patterns**

Rather than complicating the main decompiler loop, add a post-processing visitor that detects and transforms the pattern.

#### Phase 1: Track Enum Context in Switch

When processing `EnumIndex`:
1. Note that the destination register holds an enum variant index
2. Store the source enum type for later lookup

When processing `Switch`:
1. Check if the switch argument is an EnumIndex result
2. If so, mark the switch as an "enum switch" with the source enum type

**Files to modify:**
- `lib.rs`: Track enum context in `DecompilerState`
- `scopes.rs`: Add enum type to `ScopeData::Switch`

#### Phase 2: Map Case Indices to Constructor Names

In the Bytecode, enum constructors are stored with their indices:

```rust
// In hlbc/src/types.rs
pub struct EnumConstruct {
    pub name: Str,      // "None", "Some", "Red", etc.
    pub params: Vec<...>,
}
```

During switch formatting:
1. Look up the enum type
2. Get constructor name by index: `enum.constructs[case_index].name`
3. Output `case None:` instead of `case 0:`

**Files to modify:**
- `fmt.rs`: Enhanced switch formatting for enum switches
- May need to pass `Bytecode` reference to access enum metadata

#### Phase 3: Extract Field Bindings

Detect `EnumField` opcodes at the start of case bodies:
1. The `construct` field tells us which variant we're extracting from
2. The `field` tells us which parameter (0, 1, 2...)
3. The destination register becomes a binding variable

Transform:
```haxe
case 1:
    v = opt.0;  // EnumField
    return v;
```
To:
```haxe
case Some(v):
    return v;
```

**Implementation options:**
1. **AST transformation**: Post-process the AST to detect and rewrite
2. **During decompilation**: Track EnumField and merge into case pattern

Option 1 is cleaner - do the reconstruction in `post.rs` as a visitor.

### Detailed Implementation Plan

#### Step 1: Add Enum Tracking to DecompilerState

```rust
// In lib.rs
struct DecompilerState<'c> {
    // ... existing fields ...

    /// Registers that hold EnumIndex results, mapped to their source enum type
    enum_index_regs: HashMap<Reg, RefType>,
}
```

When handling `EnumIndex`:
```rust
&Opcode::EnumIndex { dst, value } => {
    // Track that dst now holds an enum index for the type of `value`
    let enum_type = f.regtype(value);
    state.enum_index_regs.insert(dst, enum_type);

    // Still emit the expression for now
    state.push_expr(i, dst, Expr::Field(...));
}
```

#### Step 2: Enhance Switch Scope with Enum Info

```rust
// In scopes.rs
ScopeData::Switch {
    arg: Expr,
    offsets: Vec<usize>,
    cases: Vec<(Vec<usize>, Vec<Statement>)>,
    enum_type: Option<RefType>,  // NEW: if switching on enum
}
```

When pushing switch:
```rust
// In lib.rs, Opcode::Switch handler
let enum_type = match state.expr(*reg) {
    Expr::Field(box inner, field) if field == "constructorIndex" => {
        // Look up the enum type from the inner expression
        state.enum_index_regs.get(reg).copied()
    }
    _ => None
};
state.scopes.push_switch(len, arg, offsets, enum_type);
```

#### Step 3: New AST Variant for Enum Switch

```rust
// In ast.rs
pub enum Statement {
    // ... existing ...

    /// Enum pattern matching switch
    EnumSwitch {
        arg: Expr,
        enum_type: RefType,
        default: Vec<Statement>,
        /// Cases: (constructor_index, bindings, statements)
        cases: Vec<(usize, Vec<Str>, Vec<Statement>)>,
    },
}
```

#### Step 4: Post-Processing Visitor

Create a new visitor in `post.rs`:

```rust
struct EnumSwitchReconstructor<'a> {
    code: &'a Bytecode,
}

impl Visitor for EnumSwitchReconstructor<'_> {
    fn visit_statement(&mut self, stmt: &mut Statement) {
        if let Statement::Switch { arg, cases, enum_type: Some(ty), .. } = stmt {
            // Transform to EnumSwitch
            // 1. For each case, extract EnumField bindings from start of body
            // 2. Map case index to constructor name
            // 3. Replace with Statement::EnumSwitch
        }
    }
}
```

#### Step 5: Format EnumSwitch

```rust
// In fmt.rs
Statement::EnumSwitch { arg, enum_type, cases, .. } => {
    "switch ("{disp!(arg)}") {\n"
    for (idx, bindings, stmts) in cases {
        let construct = &code[*enum_type].constructs[*idx];
        {indent}"case "{construct.name}
        if !bindings.is_empty() {
            "("{bindings.join(", ")}")"
        }
        ":\n"
        // ... format stmts
    }
}
```

### Complexity Assessment

| Component | Effort | Risk |
|-----------|--------|------|
| Enum tracking in DecompilerState | Low | Low |
| Enhance Switch scope | Low | Low |
| New EnumSwitch AST variant | Medium | Low |
| Post-processing visitor | Medium | Medium |
| Formatting | Low | Low |
| **Total** | **Medium** | **Medium** |

### Alternative: Simpler Approach

If full pattern reconstruction is too complex, a simpler fix:

1. Just map case indices to constructor names in formatting
2. Keep the `.0` field access syntax
3. Output: `case Some: v = opt.0; return v;` (still not perfect but better)

This would require only changes to `fmt.rs` and passing enum metadata.

### Testing

1. EnumTest.hx should compile after fix
2. Output should match original semantics
3. Dead Cells decompilation should not regress

### Files to Modify

| File | Changes |
|------|---------|
| `crates/decompiler/src/lib.rs` | Add `enum_index_regs` tracking |
| `crates/decompiler/src/scopes.rs` | Add `enum_type` to Switch scope |
| `crates/decompiler/src/ast.rs` | Add `EnumSwitch` variant |
| `crates/decompiler/src/post.rs` | Add `EnumSwitchReconstructor` visitor |
| `crates/decompiler/src/fmt.rs` | Format EnumSwitch with constructor names |
