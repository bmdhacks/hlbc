class Nd {
    public var x:Float;
    public var y:Float;
    public var next:Nd;
    public var prev:Nd;

    public inline function new(x:Float, y:Float) {
        this.x = x;
        this.y = y;
    }

    public inline function ptEquals(ox:Float, oy:Float):Bool {
        return x == ox && y == oy;
    }
}

class WhileInlineBreak {
    // while-assign-null with inlined AND-chain as break condition
    // Matches: while ((node = node.prev) != null) if (point.equals(node.point)) break;
    static function searchPrev(start:Nd, px:Float, py:Float):Nd {
        var node = start;
        while ((node = node.prev) != null) {
            if (node.ptEquals(px, py)) break;
        }
        return node;
    }

    static function searchNext(start:Nd, px:Float, py:Float):Nd {
        var node = start;
        while ((node = node.next) != null) {
            if (node.ptEquals(px, py)) break;
        }
        return node;
    }

    static function main() {
        var a = new Nd(1.0, 2.0);
        var b = new Nd(3.0, 4.0);
        var c = new Nd(5.0, 6.0);
        a.next = b;
        b.prev = a;
        b.next = c;
        c.prev = b;

        var found = searchNext(a, 5.0, 6.0);
        Sys.println(found != null ? found.x : -1.0);

        found = searchPrev(c, 1.0, 2.0);
        Sys.println(found != null ? found.x : -1.0);

        found = searchNext(a, 99.0, 99.0);
        Sys.println(found != null ? found.x : -1.0);
    }
}
