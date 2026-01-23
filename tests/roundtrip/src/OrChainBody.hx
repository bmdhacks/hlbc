// Test: OR chain where shared target is an if-body, not throw/return
// Pattern from h3d.impl.TextureCache.allocTarget:
//   if (t == null || t.isDisposed() || t.width != width) { create new; }

class OrChainBody {
    var cache:Array<Texture>;
    var position:Int;

    public function new() {
        cache = [];
        position = 0;
    }

    public function allocTarget(width:Int, height:Int):Texture {
        var t = position < cache.length ? cache[position] : null;

        // OR chain with non-terminating shared target (the if-body)
        if (t == null || t.isDisposed() || t.width != width || t.height != height) {
            if (t != null) {
                t.dispose();
            }
            t = new Texture(width, height);
            cache[position] = t;
        }

        t.prepare();
        position++;
        return t;
    }

    static function main() {
        var tc = new OrChainBody();
        var t1 = tc.allocTarget(100, 100);
        trace("First alloc: " + t1.width + "x" + t1.height);

        var t2 = tc.allocTarget(100, 100);
        trace("Second alloc (reuse): " + t2.width + "x" + t2.height);

        var t3 = tc.allocTarget(200, 200);
        trace("Third alloc (resize): " + t3.width + "x" + t3.height);
    }
}

class Texture {
    public var width:Int;
    public var height:Int;
    var disposed:Bool;

    public function new(w:Int, h:Int) {
        width = w;
        height = h;
        disposed = false;
        trace("Created texture " + w + "x" + h);
    }

    public function isDisposed():Bool {
        return disposed;
    }

    public function dispose() {
        disposed = true;
        trace("Disposed texture");
    }

    public function prepare() {
        trace("Prepared texture " + width + "x" + height);
    }
}
