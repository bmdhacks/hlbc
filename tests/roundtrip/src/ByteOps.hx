class ByteOps {
    static function main() {
        // Allocate bytes
        var b = haxe.io.Bytes.alloc(10);
        Sys.println(b.length);  // 10

        // Test set/get (8-bit ops use GetI8/SetI8 opcodes)
        b.set(0, 42);
        b.set(1, 200);
        b.set(2, 255);
        Sys.println(b.get(0));  // 42
        Sys.println(b.get(1));  // 200
        Sys.println(b.get(2));  // 255

        // Value truncation (0xF756 & 0xFF = 0x56 = 86)
        b.set(3, 0xF756);
        Sys.println(b.get(3));  // 86

        // ofString creates bytes from string
        var b2 = haxe.io.Bytes.ofString("ABCD");
        Sys.println(b2.length);  // 4
        Sys.println(b2.get(0));  // 65 (A)
        Sys.println(b2.get(1));  // 66 (B)

        // toString converts back
        Sys.println(b2.toString());  // ABCD

        // blit copies bytes
        b.blit(4, b2, 0, 4);
        Sys.println(b.get(4));  // 65 (A)
        Sys.println(b.get(5));  // 66 (B)

        // sub creates a slice
        var sub = b2.sub(1, 2);
        Sys.println(sub.toString());  // BC
    }
}
