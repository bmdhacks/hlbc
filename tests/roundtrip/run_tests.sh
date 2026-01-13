#!/bin/bash
#
# Round-trip test script for hlbc decompiler
# Usage: ./run_tests.sh [test_name]
#   - No args: run all tests
#   - With arg: run specific test (e.g., ./run_tests.sh Loop)
#

# Don't use set -e because arithmetic (( x++ )) returns 1 when x=0

# Paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HLBC_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
HAXE="/home/bmd/haxe_heaps_gles/haxe/haxe"
HLBC="$HLBC_ROOT/target/debug/hlbc"
HL="/home/bmd/haxe_heaps_gles/hashlink/build/bin/hl"

# Directories
SRC_DIR="$SCRIPT_DIR/src"
BIN_DIR="$SCRIPT_DIR/bin"
TMP_DIR="$SCRIPT_DIR/tmp"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
SKIPPED=0

# Check dependencies
check_deps() {
    local missing=0
    if [ ! -x "$HAXE" ]; then
        echo -e "${RED}ERROR: Haxe compiler not found at $HAXE${NC}"
        missing=1
    fi
    if [ ! -x "$HLBC" ]; then
        echo -e "${RED}ERROR: HLBC not found at $HLBC${NC}"
        echo "Run: cargo build -p hlbc-cli"
        missing=1
    fi
    if [ ! -x "$HL" ]; then
        echo -e "${RED}ERROR: HashLink VM not found at $HL${NC}"
        missing=1
    fi
    if [ $missing -eq 1 ]; then
        exit 1
    fi
}

# Run a single test
run_test() {
    local src_file="$1"
    local name=$(basename "$src_file" .hx)
    local hl_file="$BIN_DIR/${name,,}.hl"  # lowercase
    local rt_file="$BIN_DIR/${name,,}_rt.hl"
    local decompiled_dir="$TMP_DIR/${name,,}_decompiled"
    local single_dir="$TMP_DIR/${name,,}_single"

    echo -n "Testing $name... "

    # Step 1: Compile original
    if ! $HAXE -cp "$SRC_DIR" --main "$name" -hl "$hl_file" --debug 2>/dev/null; then
        echo -e "${YELLOW}SKIP${NC} (compile failed)"
        ((SKIPPED++))
        return
    fi

    # Step 2: Decompile
    rm -rf "$decompiled_dir"
    if ! $HLBC "$hl_file" --decompile-all -o "$decompiled_dir" >/dev/null 2>&1; then
        echo -e "${RED}FAIL${NC} (decompile failed)"
        ((FAILED++))
        return
    fi

    # Step 3: Extract user classes (main class + any other user-defined classes/enums)
    rm -rf "$single_dir"
    mkdir -p "$single_dir"
    if [ ! -f "$decompiled_dir/$name.hx" ]; then
        echo -e "${RED}FAIL${NC} (decompiled $name.hx not found)"
        ((FAILED++))
        return
    fi
    # Copy main class
    cp "$decompiled_dir/$name.hx" "$single_dir/"

    # Known stdlib types to skip (these would conflict with Haxe's stdlib)
    SKIP_TYPES="String.hx Int.hx Float.hx Bool.hx Array.hx Type.hx Dynamic.hx Null.hx Void.hx"
    SKIP_TYPES="$SKIP_TYPES Std.hx StringBuf.hx Date.hx Sys.hx SysError.hx Math.hx Reflect.hx"
    SKIP_TYPES="$SKIP_TYPES EReg.hx Xml.hx IntIterator.hx Bytes.hx"

    # Copy all user classes from root directory (not in subdirs like haxe/, hl/)
    for f in "$decompiled_dir"/*.hx; do
        base=$(basename "$f")
        # Skip: underscore files (_*.hx are compiler metadata), main class (already copied), stdlib types
        if [[ ! "$base" =~ ^_ ]] && [ "$base" != "$name.hx" ] && [[ ! " $SKIP_TYPES " =~ " $base " ]]; then
            # This is likely a user class - copy it
            cp "$f" "$single_dir/"
        fi
    done

    # Step 4: Recompile
    if ! $HAXE -cp "$single_dir" --main "$name" -hl "$rt_file" 2>/dev/null; then
        echo -e "${RED}FAIL${NC} (recompile failed)"
        # Show the error for debugging
        $HAXE -cp "$single_dir" --main "$name" -hl "$rt_file" 2>&1 | head -5
        ((FAILED++))
        return
    fi

    # Step 5: Run and compare (with timeout to catch infinite loops)
    # Extract just the values (ignore file paths and line numbers)
    local orig_output=$(timeout 5s $HL "$hl_file" 2>&1 | sed 's/^[^:]*:[0-9]*: //' | sort)
    local rt_output=$(timeout 5s $HL "$rt_file" 2>&1 | sed 's/^[^:]*:[0-9]*: //' | sort)
    local exit_code=$?

    # Check for timeout (exit code 124)
    if [ $exit_code -eq 124 ]; then
        echo -e "${RED}FAIL${NC} (timeout - possible infinite loop)"
        ((FAILED++))
        return
    fi

    if [ "$orig_output" = "$rt_output" ]; then
        echo -e "${GREEN}PASS${NC}"
        ((PASSED++))
    else
        echo -e "${RED}FAIL${NC} (output differs)"
        echo "  Original: $(echo "$orig_output" | head -1)"
        echo "  Round-trip: $(echo "$rt_output" | head -1)"
        ((FAILED++))
    fi
}

# Build hlbc
build_hlbc() {
    echo "Building hlbc..."
    if ! (cd "$HLBC_ROOT" && cargo build -p hlbc-cli 2>&1 | tail -3); then
        echo -e "${RED}ERROR: Failed to build hlbc${NC}"
        exit 1
    fi
    echo ""
}

# Main
main() {
    echo "Round-Trip Test Suite"
    echo "====================="
    echo ""

    build_hlbc
    check_deps

    mkdir -p "$BIN_DIR" "$TMP_DIR"

    if [ -n "$1" ]; then
        # Run specific test
        if [ -f "$SRC_DIR/$1.hx" ]; then
            run_test "$SRC_DIR/$1.hx"
        else
            echo -e "${RED}Test not found: $1${NC}"
            exit 1
        fi
    else
        # Run all tests
        for src_file in "$SRC_DIR"/*.hx; do
            run_test "$src_file"
        done
    fi

    echo ""
    echo "====================="
    echo -e "Results: ${GREEN}$PASSED passed${NC}, ${RED}$FAILED failed${NC}, ${YELLOW}$SKIPPED skipped${NC}"

    if [ $FAILED -gt 0 ]; then
        exit 1
    fi
}

main "$@"
