class InlinedOr {
    public var buf:haxe.io.BytesBuffer;
    public var enabled:Bool;

    public function new() {
        buf = new haxe.io.BytesBuffer();
        enabled = true;
    }

    public function writeInt(v:Int) {
        if (!enabled)
            return;
        if (v < 0 || v >= 128) {
            buf.addByte(128);
            buf.addByte(v & 0xFF);
        } else {
            buf.addByte(v);
        }
    }

    static function main() {
        var w = new InlinedOr();
        w.writeInt(42);
        w.writeInt(200);
        w.writeInt(-1);
        Sys.println(w.buf.length);
    }
}
