class BytesBufferOps {
    public var buf:haxe.io.BytesBuffer;

    public function new() {
        buf = new haxe.io.BytesBuffer();
    }

    public function writeHeader(magic:Int, version:Int) {
        buf.addInt32(magic);
        buf.addByte(version);
    }

    static function main() {
        var w = new BytesBufferOps();
        w.writeHeader(0x48454150, 1);
        Sys.println(w.buf.getBytes().length);
    }
}
