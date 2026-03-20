class OrIfElse {
    public var buf:Array<Int>;
    public var enabled:Bool;

    public function new() {
        buf = [];
        enabled = true;
    }

    public function writeInt(v:Int) {
        if (!enabled)
            return;
        if (v < 0 || v >= 128) {
            buf.push(128);
            buf.push(v);
        } else {
            buf.push(v);
        }
    }

    static function main() {
        var w = new OrIfElse();
        w.writeInt(42);
        w.writeInt(200);
        w.writeInt(-1);
        Sys.println(w.buf.length);
    }
}
