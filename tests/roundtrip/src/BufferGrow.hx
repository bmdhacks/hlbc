class BufferGrow {
    public var a:haxe.ds.Vector<Float>;
    public var b:haxe.ds.Vector<Float>;
    public var c:haxe.ds.Vector<Float>;

    public function new() {
        a = new haxe.ds.Vector(0);
        b = new haxe.ds.Vector(0);
    }

    public function grow(na:Int, nb:Int, nc:Int) {
        if (a.length < na)
            a = new haxe.ds.Vector(na);
        if (b.length < nb)
            b = new haxe.ds.Vector(nb);
        if (nc > 0 && (c == null || c.length < nc))
            c = new haxe.ds.Vector(nc);
    }

    static function main() {
        var bg = new BufferGrow();
        bg.grow(5, 10, 3);
        Sys.println(bg.a.length);
        Sys.println(bg.b.length);
        Sys.println(bg.c.length);
    }
}
