class InlineInLoop {
    public var value:Float;
    public var next:InlineInLoop;
    public var x:Float;
    public var y:Float;

    public function new(v:Float, px:Float, py:Float) {
        value = v;
        x = px;
        y = py;
    }

    public inline function pointEquals(ox:Float, oy:Float):Bool {
        return x == ox && y == oy;
    }

    // while loop with inlined AND-chain condition in body
    static function findByPoint(start:InlineInLoop, px:Float, py:Float):InlineInLoop {
        var node = start;
        while ((node = node.next) != null) {
            if (node.pointEquals(px, py)) {
                return node;
            }
        }
        return null;
    }

    // for loop with inlined ternary in body
    static function clampedSum(values:Array<Float>, lo:Float, hi:Float):Float {
        var sum = 0.0;
        for (i in 0...values.length) {
            var v = values[i];
            var clamped = v < lo ? lo : (v > hi ? hi : v);
            sum += clamped;
        }
        return sum;
    }

    // nested: inlined AND-chain inside if-else inside loop
    static function findNearest(start:InlineInLoop, px:Float, py:Float):InlineInLoop {
        var best:InlineInLoop = null;
        var bestDist = 1e18;
        var node = start;
        while (node != null) {
            if (!node.pointEquals(px, py)) {
                var dx = node.x - px;
                var dy = node.y - py;
                var d = dx * dx + dy * dy;
                if (d < bestDist) {
                    bestDist = d;
                    best = node;
                }
            }
            node = node.next;
        }
        return best;
    }

    static function main() {
        var a = new InlineInLoop(1.0, 0.0, 0.0);
        var b = new InlineInLoop(2.0, 3.0, 4.0);
        var c = new InlineInLoop(3.0, 5.0, 6.0);
        a.next = b;
        b.next = c;

        var found = findByPoint(a, 3.0, 4.0);
        Sys.println(found != null ? found.value : -1.0);

        var arr = [1.5, -3.0, 7.0, 2.5];
        Sys.println(clampedSum(arr, 0.0, 5.0));

        var nearest = findNearest(a, 4.0, 5.0);
        Sys.println(nearest != null ? nearest.value : -1.0);
    }
}
