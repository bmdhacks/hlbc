class LinkedListLoop {
    public var value:Float;
    public var next:LinkedListLoop;
    public var prev:LinkedListLoop;

    public function new(v:Float) {
        value = v;
    }

    static function findForward(start:LinkedListLoop, target:Float):LinkedListLoop {
        var node = start;
        while ((node = node.next) != null) {
            if (target < node.value) {
                return node.prev;
            }
        }
        return null;
    }

    static function findBackward(start:LinkedListLoop, target:Float):LinkedListLoop {
        var node = start;
        while ((node = node.prev) != null) {
            if (target >= node.value) {
                return node;
            }
        }
        return null;
    }

    static function main() {
        var a = new LinkedListLoop(1.0);
        var b = new LinkedListLoop(2.0);
        var c = new LinkedListLoop(3.0);
        a.next = b;
        b.prev = a;
        b.next = c;
        c.prev = b;
        var result = findForward(a, 2.5);
        if (result != null) {
            Sys.println(result.value);
        }
        result = findBackward(c, 1.5);
        if (result != null) {
            Sys.println(result.value);
        }
    }
}
