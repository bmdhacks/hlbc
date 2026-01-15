class BytesTest {
    static function main() {
        // Get embedded resource (uses Bytes opcode)
        var b = haxe.Resource.getBytes("testdata");
        if (b != null) {
            Sys.println(b.length);
            Sys.println(b.getString(0, b.length));
        } else {
            Sys.println("no resource");
        }
    }
}
