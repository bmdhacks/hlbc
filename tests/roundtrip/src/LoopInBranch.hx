class LoopInBranch {
    public var value:Float;
    public var next:LoopInBranch;
    public var prev:LoopInBranch;
    public var search:LoopInBranch;

    public function new(v:Float) {
        value = v;
    }

    public function locate(x:Float):LoopInBranch {
        var node = search;
        if (x < node.value) {
            while ((node = node.prev) != null) {
                if (x >= node.value) {
                    search = node;
                    return node;
                }
            }
        } else {
            while ((node = node.next) != null) {
                if (x < node.value) {
                    search = node.prev;
                    return node.prev;
                }
            }
        }
        return null;
    }

    static function main() {
        var a = new LoopInBranch(1.0);
        var b = new LoopInBranch(2.0);
        var c = new LoopInBranch(3.0);
        a.next = b;
        b.prev = a;
        b.next = c;
        c.prev = b;
        a.search = b;
        var result = a.locate(2.5);
        if (result != null) {
            Sys.println(result.value);
        } else {
            Sys.println("null");
        }
    }
}
