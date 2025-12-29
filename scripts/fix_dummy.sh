#!/bin/bash
# fix_dummy.sh - Iteratively fix Dummy.hx compilation errors
#
# This script attempts to compile Dummy.hx and automatically comments out
# types that fail to compile (private types, missing type parameters, etc.)
#
# Usage:
#   1. Generate Dummy.hx with: hl-substitute target.hl source.hl --gen-hxml -o /tmp/matched.hxml
#   2. cd to the directory containing Dummy.hx
#   3. Run this script: /path/to/fix_dummy.sh
#
# The script will keep trying until compilation succeeds or it hits an
# unrecognized error pattern.

HXML_FILE="${1:-/tmp/matched.hxml}"

if [ ! -f "$HXML_FILE" ]; then
    echo "Error: HXML file not found: $HXML_FILE"
    echo "Usage: $0 [path/to/matched.hxml]"
    exit 1
fi

if [ ! -f "Dummy.hx" ]; then
    echo "Error: Dummy.hx not found in current directory"
    echo "Run this script from the directory containing Dummy.hx"
    exit 1
fi

echo "Fixing Dummy.hx compilation errors..."
echo "Using HXML: $HXML_FILE"
echo ""

iteration=0
while true; do
    iteration=$((iteration + 1))
    OUTPUT=$(haxe "$HXML_FILE" 2>&1)

    if [ $? -eq 0 ]; then
        echo ""
        echo "Compilation successful after $iteration iterations!"
        break
    fi

    # Try to extract the problematic type from various error patterns
    FAILED_TYPE=""

    # Pattern 1: "Type not found : h2d.FontType"
    FAILED_TYPE=$(echo "$OUTPUT" | grep -oP "Type not found : \K[^\s]+")

    # Pattern 2: "Not enough type parameters for h3d.pass.ScreenFx"
    if [ -z "$FAILED_TYPE" ]; then
        FAILED_TYPE=$(echo "$OUTPUT" | grep -oP "Not enough type parameters for \K[^\s]+")
    fi

    # Pattern 3: Look for Class<something> in the error line
    if [ -z "$FAILED_TYPE" ]; then
        FAILED_TYPE=$(echo "$OUTPUT" | grep -oP "Class<\K[^>]+")
    fi

    # Pattern 4: "module X does not define type Y"
    if [ -z "$FAILED_TYPE" ]; then
        FAILED_TYPE=$(echo "$OUTPUT" | grep -oP "does not define type \K[^\s]+")
    fi

    # Pattern 5: Generic "Unknown identifier" or similar
    if [ -z "$FAILED_TYPE" ]; then
        # Try to extract from the line context
        ERROR_LINE=$(echo "$OUTPUT" | grep -oP "Dummy.hx:\d+:" | head -1 | grep -oP "\d+")
        if [ -n "$ERROR_LINE" ]; then
            FAILED_TYPE=$(sed -n "${ERROR_LINE}p" Dummy.hx | grep -oP "Class<\K[^>]+")
        fi
    fi

    if [ -z "$FAILED_TYPE" ]; then
        echo ""
        echo "Unhandled error after $iteration iterations:"
        echo "$OUTPUT"
        echo ""
        echo "Please fix Dummy.hx manually and re-run."
        exit 1
    fi

    echo "[$iteration] Commenting out: $FAILED_TYPE"

    # Escape special regex characters in the type name
    ESCAPED_TYPE=$(echo "$FAILED_TYPE" | sed 's/[.[\*^$()+?{|\\]/\\&/g')

    # Comment out the line containing this type
    sed -i "s|^\([^/].*Class<${ESCAPED_TYPE}>.*\)$|//\1|" Dummy.hx
done

echo ""
echo "Done! You can now run: haxe $HXML_FILE"
